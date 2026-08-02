# Research: Turning an OCI Image into a Bootable microVM Rootfs (the "Image Factory" Problem)

**Date**: 2026-08-01 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (core findings) / Medium (vendor-sourced §5) | **Sources**: 41

> **Scope note.** This document is about the **workload** image path: taking a
> Dockerfile-built OCI image and making a Cloud Hypervisor microVM boot it. It is
> **not** about the node OS image factory (Talos-style schematic → bootable
> appliance), which is covered separately in
> `docs/research/image-factory.md`.

## Executive Summary

**Every mature implementation splits the problem in two, and this is the single most
important structural finding.** The VM's own rootfs (kernel + init + agent) and the
*workload's* OCI rootfs are separate artifacts arriving by separate mechanisms. Kata,
firecracker-containerd, and AWS Lambda all do this. Conflating them — "boot the
container image as the VM" — is the design error that makes the problem look harder
than it is. Once split, the workload half reduces to: **flatten the OCI layers into
one filesystem image on the host, hand the guest a block device.** That is literally
what AWS Lambda does (layers unpacked "onto an ext4 filesystem, using a modified
filesystem implementation that performs all operations deterministically"), and
everything else in Lambda's pipeline — 512 KiB chunking, convergent encryption, a
three-tier cache, erasure coding — is an optimisation *over that flat artifact*, driven
by multi-tenancy and fleet scale that a single-tenant appliance does not have.

**The field is converging away from userspace filesystem daemons in the hot path, and
three independent sources say so.** Nydus measured FUSE at 0.78× native on a kernel
compile and rebuilt its format around in-kernel EROFS; the Lambda team wrote "we are
moving away from FUSE for this application"; Modal had to raise FUSE read-ahead 250×
(128 KB → 32 MB) to make it competitive. For Cloud Hypervisor specifically the case is
stronger still: **virtio-fs DAX — the whole performance argument for virtio-fs — does
not exist there.** CH's own docs state "the DAX feature is not stable yet from a daemon
standpoint, it is not available in Cloud Hypervisor", and DAX remains unimplemented in
the Rust virtiofsd. A designer who reads Kata's "virtio-fs is the default" and infers
"virtio-fs is the fast path on CH" will budget for page-cache sharing that is not
available. Kata itself retains a block-device opt-in precisely because virtio-fs loses
to block storage on rootfs-shaped I/O.

**The most decisive evidence is six months old and points at virtio-blk.**
CVE-2026-24834 (published 2026-02-19, CVSS 9.3) was a container-to-guest escape in
exactly the sophisticated path a first-principles designer would reach for: Kata
DAX-mapping a read-only guest image over **virtio-pmem** on Cloud Hypervisor. The
remediation in Kata 3.27.0 "changes the VM rootfs driver from `virtio-pmem` to
`virtio-blk-pci` for Cloud Hypervisor configurations". The most experienced team in
this space tried the clever option on this exact VMM, got burned, and retreated to the
boring one — which is also what CH's own device model documentation recommends
("virtio-blk… is usually used to boot the operating system running in the VM").
Accordingly, the recommendation for Overdrive's first slice is deliberately boring:
**flatten the OCI image to `ext4`, attach as `virtio-blk`, per-launch writable copy via
host reflink, plus a ~200-line PID-1 init that reports exit status over vsock** — the
agent is required, because Overdrive's existing restart/backoff model is driven by
observed exit codes that an agentless VM cannot report. The end state is **erofs +
in-guest overlayfs**, the only surveyed format offering cross-VM page-cache sharing on
one machine, gated on measurement.

## Research Methodology

**Search Strategy**: Primary-source-first. For each system I went to the project's own
design documents in its source tree (Kata `docs/design/`, Cloud Hypervisor `docs/`,
firecracker-containerd `docs/`, containerd snapshotter docs, kernel.org filesystem
docs) rather than to summaries. The AWS Lambda mechanism was taken from the
peer-reviewed USENIX ATC '23 paper. Vendor engineering blogs were used only for
platforms with no other public account, and are flagged as such. Commit histories and
security advisories were read directly to establish 2026 currency.

**Source Selection**: Types: academic (1 peer-reviewed paper), official project
documentation (18), vendor engineering blogs (6), issue trackers / mailing lists (4),
security advisories (2), independent technical journalism (2). Minimum reputation:
Medium, with Medium-tier sources used only where cross-referenced or explicitly flagged.

**Verification approach**: Every load-bearing claim was cross-referenced against at
least one independent source, with two deliberate exceptions noted inline (CH's
`virtiofs-root.md`, and the Kata shim-v2 description — both single authoritative
primary sources). Two circularity risks are called out explicitly: the CNCF and
Alibaba Cloud nydus articles share authors and are **not** independent; the EROFS FAQ's
comparison against SquashFS is self-authored and is balanced against an upstream
mailing-list thread reporting the opposite result.

**Quality standards applied**: Vendor numbers without stated methodology are flagged
inline rather than quietly reported. Claims I could not source are marked
**UNVERIFIED** rather than inferred. Where I synthesised rather than quoted (the format
comparison table, the "minimal agent" table), the synthesis is labelled as
interpretation.

## Findings

### 1. Kata Containers — the reference OCI→VM path

Kata is the most mature production OCI→VM path and the single best source of
"what did we learn" evidence. Its key structural decision: **the guest image and
the container image are two separate things.** The guest image boots the VM; the
container rootfs is layered in afterwards.

#### 1.1 Two guest-image shapes: rootfs image vs initrd

**Evidence**: "Kata Containers supports both an initrd and rootfs based minimal
guest image." The **rootfs image** boot sequence is: runtime launches hypervisor →
hypervisor boots image with the guest kernel → kernel starts `systemd` as PID 1 →
`systemd` launches the agent → agent creates the container environment. The
**initrd** is "a compressed `cpio(1)` archive" where "the agent is the init daemon
(PID 1)". Defaults differ per shape: the rootfs image defaults to **Ubuntu +
systemd** on x86_64; the initrd defaults to **Alpine Linux with the Kata agent as
init**, chosen for being "security hardened and tiny C library". Both are produced
by the **`osbuilder`** tool.
**Source**: [kata-containers `docs/design/architecture/guest-assets.md`](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/guest-assets.md) — Accessed 2026-08-01
**Confidence**: High (primary source, project's own design doc)
**Verification**: [Kata Containers Architecture](https://kata-containers.github.io/kata-containers/design/architecture/) — Accessed 2026-08-01; [kata-containers `documentation/design/architecture.md`](https://github.com/kata-containers/documentation/blob/master/design/architecture.md) — Accessed 2026-08-01
**Analysis (interpretation)**: The size delta is the decisive axis — initrd is
"10MB+" vs rootfs image "100MB+" per the older architecture doc. The initrd shape
collapses the boot chain: no systemd, no init, the agent *is* PID 1. For an
appliance that controls its own guest image and does not need a general-purpose
guest userspace, the initrd shape is strictly simpler. Note this is the **guest**
image decision and is orthogonal to how the *workload's* OCI rootfs arrives.

#### 1.2 The container rootfs arrives separately — virtio-fs by default

**Evidence**: "If a block-based graph driver is _not_ configured, a `virtio-fs`
(`VIRTIO`) overlay filesystem mount point is used to _share_ the workload image
instead." "The hypervisor mounts the OCI bundle, using virtio FS, into a container
specific directory inside the VM's rootfs." The environments table lists the
container rootfs device type as **`kataShared` (virtio FS)**.
**Source**: [Kata Storage design](https://kata-containers.github.io/kata-containers/design/architecture/storage/) — Accessed 2026-08-01
**Confidence**: High
**Verification**: [Kata Architecture](https://kata-containers.github.io/kata-containers/design/architecture/) — Accessed 2026-08-01
**Analysis (interpretation)**: This is the load-bearing architectural split. The VM
boots from its *own* rootfs (image or initrd); the *workload's* OCI bundle is a
host directory handed in over virtio-fs and mounted at a container-specific path.
The host-side snapshotter (overlayfs) does the layer assembly; the guest sees a
single already-composed directory. **This means the "unpack OCI layers" problem is
solved on the host by existing containerd machinery, not in the guest.**

#### 1.3 The 9pfs → virtio-fs transition and why virtio-fs won

**Evidence**: 9pfs was the default filesystem-sharing mechanism as of Kata 1.7;
virtio-fs became the default in Kata 2.0. The stated rationale: "virtio-fs aims to
take advantage of the co-location between the virtual machine and the hypervisor in
order to achieve local file system semantics and improve performance", including
using "Linux Direct Access (DAX) to access file contents directly from the host page
cache, which reduces communication with the file server and avoids duplicating data
into each sandbox VM."
**Source**: [How to use virtio-fs with Kata](https://github.com/kata-containers/documentation/blob/master/how-to/how-to-use-virtio-fs-with-kata.md) — Accessed 2026-08-01
**Confidence**: Medium-High (two sources agree on the transition and rationale; the
*measured* performance delta is less well-sourced — see 1.4)
**Verification**: [Kata Containers 1.7.0 Release Highlights](https://medium.com/kata-containers/kata-containers-1-7-0-release-highlights-9e07ddbe737e) — Accessed 2026-08-01; [Exploration and Practice of Performance Tuning for Kata Containers 2.0](https://medium.com/kata-containers/exploration-and-practice-of-performance-tuning-for-kata-containers-2-0-85055d29e8b5) — Accessed 2026-08-01
**Analysis (interpretation)**: The two claimed wins are **POSIX compliance** and
**page-cache sharing via DAX**. The POSIX win is unambiguous and is arguably the
real reason 9pfs had to go (9p's semantics broke real workloads). The DAX win is
conditional — see the DAX status finding in §4.

#### 1.4 …but virtio-fs is *not* faster than a block device for the rootfs

**Evidence**: Kata's own issue tracker and the virtio-fs project's tracker carry
sustained reports that block storage beats virtio-fs for rootfs-shaped I/O: "for
many workloads in ci/cd like `docker build` or decompressing big gz files the
performance of virtio-fs is not close to block storage." A Kata runtime issue titled
"Poor qemu-virtiofs performance in benchmarks" records early virtiofs measuring
*worse* than 9p.
**Source**: [virtio-fs/qemu issue #20 — "Performance of virtio-fs vs. block storage for kata dind use-case"](https://gitlab.com/virtio-fs/qemu/-/work_items/20) — Accessed 2026-08-01
**Confidence**: Medium (issue-tracker discussion, not a controlled benchmark; the
early "worse than 9p" result was subsequently addressed)
**Verification**: [kata-containers/runtime issue #2138](https://github.com/kata-containers/runtime/issues/2138) — Accessed 2026-08-01; [Disk I/O Performance of Kata Containers (StackHPC)](https://www.stackhpc.com/images/IO-Performance-of-Kata-Containers-TheNewStack.pdf) — Accessed 2026-08-01
**Analysis (interpretation)**: **This is the most important corrective in the whole
Kata story.** virtio-fs became the default for *compatibility and flexibility*, not
because it is the fastest path. Kata retains `disable_block_device_use = false` as
an explicit opt-in precisely so that deployments that care about rootfs I/O can pass
a real block device: "The `devicemapper` `snapshotter` uses dedicated block devices
rather than formatted filesystems, and operates at the block level rather than the
file level." Do not read "Kata defaults to virtio-fs" as "virtio-fs is the
performance answer."

#### 1.5 kata-agent: ttRPC over vsock

**Evidence**: "The runtime is responsible for starting the hypervisor and it's VM,
and communicating with the agent using a **ttRPC based protocol over a VSOCK
socket** that provides a communications link between the VM and the host." The API
is request/response over that channel (e.g. "the runtime will signal the agent by
sending it a `DestroySandbox` ttRPC API request").
**Source**: [Kata Containers Architecture](https://kata-containers.github.io/kata-containers/design/architecture/) — Accessed 2026-08-01
**Confidence**: High
**Verification**: [kata-containers `docs/design/architecture.md`](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture.md) — Accessed 2026-08-01
**Analysis (interpretation)**: ttRPC is a protobuf RPC framework designed as a
low-overhead alternative to gRPC for exactly this constrained-transport case — it
drops HTTP/2 framing. The choice of **vsock** as transport is the near-universal
pattern (confirmed across every implementation surveyed — see §8).

#### 1.6 containerd shim v2 integration

**Evidence**: "Rather than calling the runtime multiple times for each new
container, the shimv2 architecture runs a single instance of the runtime binary (for
any number of containers)." The container manager "creates a socket and passes it to
the shimv2 runtime… a bi-directional communication channel that uses a gRPC based
protocol."
**Source**: [Kata Containers Architecture](https://kata-containers.github.io/kata-containers/design/architecture/) — Accessed 2026-08-01
**Confidence**: High (single authoritative primary source; project's own design doc)
**Analysis (interpretation)**: Note the two-protocol structure: **gRPC host-side**
(containerd ↔ shim) and **ttRPC guest-side** (shim ↔ agent). Overdrive has no
containerd, so the host-side half is irrelevant; only the guest-side agent protocol
transfers.

#### 1.7 nydus: lazy-loading the image instead of materialising it

**Evidence**: Kata's nydus design substitutes `nydusd` for `virtiofsd` — "we can use
`nydusd` in place of `virtiofsd` and mount `nydus` image to guest in the meanwhile."
Motivation, quoted in the design doc: "time to take for pull operation accounts for
**76% of container startup time but only 6.4% of that data is read**." The guest
composes `overlay(lowerdir=rafs, upperdir=snapshotdir/fs, workdir=snapshotdir/work)`
— i.e. the lazily-loaded RAFS layer is the read-only lower, with a writable upper.
**Source**: [kata-containers `docs/design/kata-nydus-design.md`](https://github.com/kata-containers/kata-containers/blob/main/docs/design/kata-nydus-design.md) — Accessed 2026-08-01
**Confidence**: High for the design; the 76%/6.4% figure is a **secondary citation**
inside the design doc (originally from the FAST '16 Slacker paper) — treat the
number as indicative, not as Kata's own measurement.
**Verification**: [containerd/nydus-snapshotter](https://github.com/containerd/nydus-snapshotter) — Accessed 2026-08-01; [Nydus project site](https://nydus.dev/) — Accessed 2026-08-01
**Analysis (interpretation)**: The overlay composition here is the canonical answer
to "how do you get a writable layer over a read-only image": read-only lower +
writable upper, composed **in the guest**. That pattern is format-independent and
transfers directly to squashfs/erofs/virtiofs.

#### 1.8 RAFS v6 / EROFS-over-fscache: measured numbers

**Evidence**: RAFS v5 was a userspace (FUSE/virtiofs) format and hit "excessive
system call overhead", "frequent kernel/userspace context switching", "buffer
copying overhead", and "DAX window resource contention in high-density scenarios".
RAFS v6 is "compatible with the in-kernel EROFS filesystem"; EROFS-over-fscache
merged into **Linux 5.19** and is described as "the first native in-kernel solution
for container images". Measured (fio, 4K blocksize):

| Workload | Loop | Fscache | FUSE | Native ext4 |
|---|---|---|---|---|
| Read IOPS | 240K | 227K | 191K | 267K |
| Read BW | 982 MB/s | 931 MB/s | 764 MB/s | 1093 MB/s |
| Randread IOPS | 8.7K | 9.5K | 7.6K | 10.1K |

Metadata (`tar` over a large file set): native ext4 1.04 s, fscache 0.570 s (1.82×
*faster* than native ext4), FUSE 3.2 s. Linux kernel compile (`-j16`): native ext4
156 s, fscache 156 s (parity), FUSE 200 s (0.78×).
**Source**: [The Evolution of the Nydus Image Acceleration (CNCF)](https://www.cncf.io/blog/2022/11/15/the-evolution-of-the-nydus-image-acceleration/) — Accessed 2026-08-01
**Confidence**: Medium-High. CNCF is a high-reputation publisher, but the post is
authored by the Nydus/Alibaba team — **vendor-adjacent numbers**; the benchmark
tool (fio) and blocksize (4K) are stated but the hardware and kernel are not fully
specified. The `tar` result beating native ext4 is plausible (EROFS packs metadata
more densely, read-only) but is exactly the kind of headline figure to treat with
caution.
**Verification**: [Faster Container Image Loading Speed with Nydus, RAFS, and EROFS (Alibaba Cloud)](https://www.alibabacloud.com/blog/faster-container-image-loading-speed-with-nydus-rafs-and-erofs_599012) — Accessed 2026-08-01 (**note**: fetch returned empty content; cited as a pointer only, not as independent verification); [Dragonfly — Evolution of Nydus](https://d7y.io/blog/2022/06/06/evolution-of-nydus/) — Accessed 2026-08-01
**Analysis (interpretation)**: The load-bearing finding is the **FUSE column**: FUSE
(and therefore userspace virtiofs-style paths) costs ~22–28% on realistic workloads
vs. an in-kernel filesystem. This is the quantitative case against "just virtiofs
everything", and the quantitative case *for* a block-device-backed read-only
in-kernel filesystem (erofs/squashfs/ext4) when the image is already local.

### 2. firecracker-containerd — the snapshotter/devmapper model

#### 2.1 Architecture: four pieces, two of them host-side

**Evidence**: The components are (a) a **control plugin** "compiled in to the
containerd binary, which requires us to build a specialized containerd binary"; (b)
a **runtime shim**, "an out-of-process shim communicating over ttrpc" linking
containerd to both the Firecracker VMM and the in-VM agent; (c) an **in-VM agent**
responsible "for acting on control instructions received from the runtime, for
emitting event and metric information to the runtime, and for proxying STDIO for
container processes" — it "invokes runC through containerd's shim to create standard
Linux containers" inside the microVM; and (d) a **root filesystem image builder**
that "constructs a firecracker microVM root filesystem containing runc and the
firecracker-containerd agent."
**Source**: [firecracker-containerd `docs/architecture.md`](https://github.com/firecracker-microvm/firecracker-containerd/blob/main/docs/architecture.md) — Accessed 2026-08-01
**Confidence**: High (primary source)
**Verification**: [firecracker-containerd README](https://github.com/firecracker-microvm/firecracker-containerd) — Accessed 2026-08-01
**Analysis (interpretation)**: Note the **nesting**: the guest runs `runc`, so the
workload is a container *inside* a VM. The guest rootfs (agent + runc) and the
workload image are, again, two separate artifacts. Same split as Kata §1.2.

#### 2.2 Container image → block device: the devmapper snapshotter

**Evidence**: "Devmapper is a `containerd` snapshotter plugin that stores snapshots
in filesystem images in a Device-mapper thin-pool." Per-layer, it creates thin
devices in the pool, formats them (ext4 or xfs by default), and mounts them.
`base_image_size` "defines how much space to allocate when creating thin device
snapshots from the base (pool) device."
**Source**: [containerd `docs/snapshotters/devmapper.md`](https://github.com/containerd/containerd/blob/main/docs/snapshotters/devmapper.md) — Accessed 2026-08-01
**Confidence**: High (primary source, containerd project docs)
**Analysis (interpretation)**: This is the canonical "OCI layers → block device"
mechanism. Device-mapper thin provisioning gives **CoW block snapshots for free**:
each container gets a thin snapshot of the image's device, so N containers off one
image cost one image's worth of blocks plus per-container deltas. That is the
block-layer equivalent of overlayfs, and it is what makes a *writable* rootfs cheap.

#### 2.3 The devmapper production caveat

**Evidence**: The loopback-device setup is "simple and suits well for development
and testing (_please note that this configuration is slow and not recommended for
production uses_)"; production is directed at Docker's `direct-lvm` mode. Requires
`dmsetup (>= 1.02.110)`.
**Source**: [containerd `docs/snapshotters/devmapper.md`](https://github.com/containerd/containerd/blob/main/docs/snapshotters/devmapper.md) — Accessed 2026-08-01
**Confidence**: High
**Analysis (interpretation)**: The devmapper path drags in a **host storage
provisioning dependency** — you must dedicate an LVM thin-pool (a real block device
or a carefully sized volume) at node install time. On an immutable appliance that is
a real cost: it becomes part of the disk layout contract, not a runtime decision.

#### 2.4 Firecracker's no-hotplug constraint (and why it matters to Cloud Hypervisor)

**Evidence**: Firecracker lacks hot-plug capability, so all block devices "need to be
attached before running the microVM (and need to know the number of drives to be
used in advance)." firecracker-containerd's workaround: reserve drive IDs upfront
with placeholder devices (`/dev/null` or sparse files), then "fake device is replaced
(via `PatchGuestDriveByID`) with real container image received as mount from
snapshotter."
**Source**: [firecracker-containerd `docs/design-approaches.md`](https://github.com/firecracker-microvm/firecracker-containerd/blob/main/docs/design-approaches.md) — Accessed 2026-08-01
**Confidence**: High (primary source)
**Analysis (interpretation)**: This is a **Firecracker-specific wart that Overdrive
does not inherit** — Cloud Hypervisor supports device hotplug (see §4). A large
fraction of firecracker-containerd's complexity is this placeholder-drive dance;
reading it as "the OCI→VM problem is inherently this complex" would be a mistake.

#### 2.5 Maintenance status as of August 2026

**Evidence**: The repository has commits through **July 16 2026** ("Update CODEOWNERS
file (#876)", "Update deps (#875)" 2026-07-14, "Upgrade to containerd v1.7.33
(#873)" 2026-07-10). The GitHub Releases page shows **"There aren't any releases
here"** — the project has never cut a tagged release. Recent activity is dominated by
dependency bumps and administrative changes rather than feature work.
**Source**: [firecracker-containerd commit history](https://github.com/firecracker-microvm/firecracker-containerd/commits/main) — Accessed 2026-08-01
**Confidence**: Medium-High for the activity dates (directly observed); **Medium**
for the characterisation "maintenance-mode". The repo is **not** archived and carries
no deprecation notice — I found no such statement.
**Verification**: [firecracker-containerd Releases](https://github.com/firecracker-microvm/firecracker-containerd/releases) — Accessed 2026-08-01; [pkg.go.dev module page](https://pkg.go.dev/github.com/firecracker-microvm/firecracker-containerd) — Accessed 2026-08-01
**Analysis (interpretation)**: Read this as **alive but not evolving**. It is still
pinned to containerd 1.7.x (not 2.x), has never shipped a release, and its recent
commits are upkeep. It is a good *reference design* and a poor *dependency*. For
Overdrive — which has no containerd at all — it is reference-only regardless.

### 3. AWS Lambda — chunked, deduplicated, convergent-encrypted container loading

This is the single most relevant published system, and it is a peer-reviewed paper
with real production numbers. Brooker et al., **USENIX ATC '23**.

#### 3.1 The mechanism, end to end

**Evidence**: OCI layers (tarballs) are "unpacked onto an **ext4 filesystem**, using a
modified filesystem implementation that performs all operations **deterministically**"
— determinism is the point, because it ensures "blocks of the filesystem that contain
unchanged files will be identical, allowing for block-level deduplication." The flat
ext4 image is then split into **fixed 512 KiB chunks**. Each chunk is encrypted under
a key derived from its own content (SHA-256 → AES-CTR, deterministic all-zero IV) —
**convergent encryption**. A manifest holds "offset, unique key, and SHA256 hash of
each chunk" and is itself encrypted under a per-customer AWS KMS key.
**Source**: [Brooker et al., "On-demand Container Loading in AWS Lambda", USENIX ATC '23 (arXiv:2305.13162)](https://ar5iv.labs.arxiv.org/html/2305.13162) — Accessed 2026-08-01
**Confidence**: High (peer-reviewed, USENIX ATC '23; authors are the system's
operators)
**Verification**: [USENIX ATC '23 program page](https://www.usenix.org/conference/atc23/presentation/brooker) — Accessed 2026-08-01 (**note**: returned HTTP 403 to automated fetch; cited as the canonical venue record); [Amazon Science publication page + PDF](https://assets.amazon.science/25/06/d2e5ea9c411c9e4d366aa2fbbca5/on-demand-container-loading-in-aws-lambda.pdf) — Accessed 2026-08-01
**Analysis (interpretation)**: **The key idea for Overdrive is the first step, not the
cryptography.** "Flatten the OCI image to a single deterministic filesystem image" is
the move. Everything downstream (chunking, dedup, caching) is an optimisation over
that flat artifact. A single-tenant appliance can adopt the flattening and skip the
convergent encryption entirely — convergent encryption exists to allow **cross-tenant**
dedup without cross-tenant plaintext exposure. With one tenant there is nothing to
defend against, and the whole crypto layer is dead weight.

#### 3.2 Chunk-size tradeoff, stated explicitly

**Evidence**: 512 KiB, and the paper states the tradeoff directly: "Smaller chunks
lead to better deduplication by minimizing false-sharing, and can accelerate loading
for workloads with highly random access patterns. Larger chunks reduce metadata size,
reduce the number of requests needed to load data."
**Source**: [arXiv:2305.13162](https://ar5iv.labs.arxiv.org/html/2305.13162) — Accessed 2026-08-01
**Confidence**: High
**Analysis (interpretation)**: Note this is **fixed-size** chunking, not
content-defined chunking (CDC/rolling-hash à la restic/borg). Fixed-size works here
*only because* the ext4 builder is deterministic — determinism removes the
insertion-shift problem that CDC exists to solve. That is an elegant trade: pay for
determinism once in the builder, get cheap fixed chunking forever.

#### 3.3 Deduplication actually measured

**Evidence**: "Approximately **80% of newly uploaded Lambda functions result in zero
unique chunks**." Of the remainder, "mean upload contains **4.3% unique chunks**, and
the median **2.5%**." Maximum observed benefit: "reducing storage by as much as
**23x**." Manifest overhead is "less than 3MiB for a 16GiB container image, or
**0.02% overhead**." Supports images "as large as 10GiB".
**Source**: [arXiv:2305.13162](https://ar5iv.labs.arxiv.org/html/2305.13162) — Accessed 2026-08-01
**Confidence**: High (production fleet measurement, stated methodology and window)
**Analysis (interpretation)**: The 80%/4.3% figures are a **multi-tenant public-cloud**
distribution — driven by the long tail of customers pushing near-identical
base images. A single-tenant appliance will see a *different* distribution, but the
underlying driver (shared base layers across a team's images) still holds and is
arguably *stronger* within one organisation. The 23× is a maximum, not a typical.

#### 3.4 Tiered cache and hit rates

**Evidence**: Three tiers — per-worker local cache (L1), AZ-level distributed cache
(L2), S3 (L3). Over one week: "median of **67% of chunks were loaded from the
on-worker cache, 32% from the AZ-level distributed cache, and the remaining 0.06%
from the backing store**." The L2 cache itself hit "median hit rate of 99.9% and 10th
percentile low hit rate over the week of 99.4%."
**Source**: [arXiv:2305.13162](https://ar5iv.labs.arxiv.org/html/2305.13162) — Accessed 2026-08-01
**Confidence**: High
**Analysis (interpretation)**: **Two-thirds of all chunk loads are served from the
local worker cache.** For a single-node appliance, L2 and L3 collapse away entirely —
the on-node cache *is* the system. This is the strongest evidence that Overdrive's
first slice should be a plain on-node content-addressed store and nothing else.

#### 3.5 Latency numbers

**Evidence**:

| Measurement | Value |
|---|---|
| L2 cache server GET (512 KiB chunk) | median < 50 μs |
| L2 cache server PUT | median 125 μs; p99 < 300 μs |
| End-to-end read, local cache hit | mode < 100 μs |
| End-to-end read, L2 hit | mode ≈ 2.75 ms |
| AZ cache hit vs S3 origin fetch | median 550 μs vs **36 ms**; p99.9 3.7 ms vs **175 ms** |

**Source**: [arXiv:2305.13162](https://ar5iv.labs.arxiv.org/html/2305.13162) — Accessed 2026-08-01
**Confidence**: High
**Analysis (interpretation)**: The ~65× median gap between a cache hit and an origin
fetch is the entire justification for the caching tier. Locally: a local-cache hit
(<100 μs) is ~27× faster than even the AZ hit.

#### 3.6 How the guest actually sees it — and the FUSE retreat

**Evidence**: A "per-function local agent" exposes a FUSE filesystem presenting "**a
block device** to the per-function Firecracker hypervisor (via FUSE), which is then
forwarded using the **existing virtio interface** into the guest, where it is
**mounted by the guest kernel**." Critically: "**We are moving away from FUSE for this
application, primarily due to this effect**", replacing it with `userfaultfd` and
`mmap` to cut context-switching overhead.
**Source**: [arXiv:2305.13162](https://ar5iv.labs.arxiv.org/html/2305.13162) — Accessed 2026-08-01
**Confidence**: High (explicit statement by the system's authors)
**Analysis (interpretation)**: Three things transfer directly:
1. **The guest sees an ordinary virtio block device with an ordinary ext4 on it.**
   No guest-side awareness of chunking, dedup, or encryption. The guest kernel needs
   zero special support. This is the cheapest possible guest contract.
2. **The demand-loading is entirely a host-side concern** — the host lies to the guest
   about the block device being fully materialised, and faults chunks in behind it.
3. **FUSE was measured as the bottleneck and is being removed.** This is the *third*
   independent corroboration of the anti-FUSE finding (with nydus §1.8 and Kata §1.4).
   Any design whose hot path is a userspace FUSE/virtiofs daemon should expect to pay
   for it.

#### 3.7 Erasure coding (noted; not applicable to a single node)

**Evidence**: **4-of-5 erasure coding** in the AZ-level cache, giving "25% storage
overhead, and a 25% increase in request rate in exchange for a significant decrease in
tail latency" — chosen to prevent hit-rate collapse during node failures and
deployments "while avoiding retry-based metastability issues."
**Source**: [arXiv:2305.13162](https://ar5iv.labs.arxiv.org/html/2305.13162) — Accessed 2026-08-01
**Confidence**: High
**Analysis (interpretation)**: Purely a distributed-cache concern. Irrelevant to a
single-node appliance; noted so it is not mistaken for part of the core mechanism.

### 4. Cloud Hypervisor — what the project itself recommends

#### 4.1 virtio-blk is the documented boot path

**Evidence**: CH's own device model doc: **virtio-blk** "exposes a block device to
the guest. **This device is usually used to boot the operating system running in the
VM.**" virtio-pmem "emulates a virtual persistent memory device that cloud-hypervisor
can e.g. boot from. Booting from a virtio-pmem device allows **bypassing the guest
page cache and improve the guest memory footprint**." virtio-fs allows "an efficient
and reliable way of sharing a filesystem between the host and the cloud-hypervisor
guest." All listed virtio devices are marked runtime-configurable.
**Source**: [cloud-hypervisor `docs/device_model.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/device_model.md) — Accessed 2026-08-01
**Confidence**: High (primary source)
**Analysis (interpretation)**: CH's documentation frames **virtio-blk as the normal
boot device** and virtio-fs as a *sharing* mechanism, not a root mechanism. This is
the opposite emphasis from Kata's container-rootfs default, and it matters: CH is
telling you the well-trodden path.

#### 4.2 virtiofs-as-root is possible but explicitly not production-ready

**Evidence**: CH ships a `virtiofs-root.md` guide (Alpine rootfs, `virtiofsd
--cache=never`, kernel cmdline pointing at the `/dev/root` tag). It carries two
caveats: "For whatever reason, it would only work with that as the tag", and the doc
describes itself as "a quick getting started guide" with "**many more steps to take
to make this a production ready, secure setup**."
**Source**: [cloud-hypervisor `docs/virtiofs-root.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/virtiofs-root.md) — Accessed 2026-08-01
**Confidence**: High (primary source; the caveat is the project's own words)
**Analysis (interpretation)**: A doc that contains "for whatever reason" and
"not production ready" is not a recommendation. Treat virtiofs-as-root on CH as
**experimental**.

#### 4.3 virtio-fs requires shared memory — with a real cost

**Evidence**: "This virtual device relies on the vhost-user protocol, which assumes
the backend (device emulation) is handled by a dedicated process running on the host.
This daemon is called `virtiofsd`." And: "**Correct functioning of `--fs` requires
`--memory shared=on`** to facilitate interprocess memory sharing."
**Source**: [cloud-hypervisor `docs/fs.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/fs.md) — Accessed 2026-08-01
**Confidence**: High
**Verification**: [Cloud Hypervisor virtio-fs docs (HTML mirror)](https://intelkevinputnam.github.io/cloud-hypervisor-docs-HTML/docs/fs.html) — Accessed 2026-08-01
**Analysis (interpretation)**: `shared=on` means the guest's entire RAM is a shared
memory mapping. This **defeats some memory-overcommit and KSM-style strategies** and
enlarges the host-side attack surface. Plus you now run **one extra host process per
VM** (`virtiofsd`) that must be sandboxed, supervised, and lifecycle-managed. For a
Rust orchestrator that would be a new supervised-process class — non-trivial.

#### 4.4 **virtiofsd DAX is NOT available in Cloud Hypervisor** (as of 2026-08)

**Evidence**: CH's own docs state flatly: "**Given the DAX feature is not stable yet
from a daemon standpoint, it is not available in Cloud Hypervisor.**" Independently,
DAX support "is not implemented in the Rust version of virtiofsd"; the C virtiofsd is
gone. Blockers cited: a guest process should get `SIGBUS` when accessing a file beyond
its bounds (possible if the host truncates a file mapped in the guest), and "virtiofsd
needs additional vhost-user commands to implement DAX, and these commands never went
upstream in QEMU."
**Source**: [cloud-hypervisor `docs/fs.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/fs.md) — Accessed 2026-08-01
**Confidence**: High for "not available in CH" (project's own primary doc).
**Medium** for the Rust-virtiofsd blockers — sourced to a Red Hat bugzilla RFE and
forum/mailing-list discussion rather than a dated release note. See **Conflicting
Information** below.
**Verification**: [Red Hat Bugzilla 1890692 — "RFE: [virtiofsd] Support DAX for faster speed"](https://bugzilla.redhat.com/show_bug.cgi?id=1890692) — Accessed 2026-08-01; [virtio-fs project site](https://virtio-fs.gitlab.io/) — Accessed 2026-08-01
**Analysis (interpretation)**: **This demolishes the main theoretical argument for
virtio-fs on Cloud Hypervisor.** The headline virtio-fs benefit — "use DAX to access
file contents directly from the host page cache… avoids duplicating data into each
sandbox VM" (§1.3) — *is not available on this VMM*. Without DAX, virtio-fs is a
FUSE-over-vhost-user protocol with a userspace daemon in the hot path, i.e. exactly
the configuration the nydus benchmarks (§1.8) and the Lambda team (§3.6) measured as
the slow one. **Any Overdrive design premised on virtiofs DAX page-cache sharing is
premised on a feature that does not exist here.**

#### 4.5 Cloud Hypervisor supports hotplug — Firecracker's placeholder dance is unnecessary

**Evidence**: CH supports hot-plug of "Disk, Network, PMEM, Vsock, and VFIO devices"
plus CPU (x86) and memory resize, via `ch-remote`:
`add-disk path=…`, `add-net tap=…`, `add-pmem file=…`, `add-vsock cid=…`,
`remove-device _disk0`.
**Source**: [cloud-hypervisor `docs/hotplug.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/hotplug.md) — Accessed 2026-08-01
**Confidence**: High
**Verification**: [cloud-hypervisor `docs/device_model.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/device_model.md) — Accessed 2026-08-01 (all virtio devices marked runtime-configurable); [kata-containers/runtime PR #2681 "clh: Enable disk block device hotplug support"](https://github.com/kata-containers/runtime/pull/2681) — Accessed 2026-08-01
**Analysis (interpretation)**: Directly cancels §2.4. Overdrive can attach the
workload rootfs at boot *or* hot-attach volumes later, with no
`PatchGuestDriveByID`-style placeholder machinery.

#### 4.6 CVE-2026-24834 — the virtio-pmem DAX rootfs path was **critically broken** on CH

**Evidence**: **CVE-2026-24834**, published **2026-02-19**, **CVSS 9.3 (Critical)**,
`CVSS:3.1/AV:L/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H`. Mechanism: "Kata boots each pod/VM by
DAX-mapping a read-only guest image from the host into the VM and telling the guest
kernel to mount the resulting `/dev/pmem*` device as its root filesystem." But "the
`virtio-pmem` probe path always registers the region as a generic pagemap that
supports asynchronous flushes, but **it never marks the region as read-only**", and on
CH "`discard_writes=on` causes the file backing the `virtio-pmem` device to be opened
read-only and mapped with `MAP_PRIVATE` rather than `MAP_SHARED`." A container user
with `CAP_MKNOD` could then write directly to guest filesystem structures. Impact:
"Container to Guest micro VM Escape (no escape to Host, no persistence of the
overwritten image)". Affected ≤ 3.26.0; **patched in 3.27.0 — and the fix "changes the
VM rootfs driver from `virtio-pmem` to `virtio-blk-pci` for Cloud Hypervisor
configurations**".
**Source**: [Kata Containers security advisory GHSA-wwj6-vghv-5p64](https://github.com/kata-containers/kata-containers/security/advisories/GHSA-wwj6-vghv-5p64) — Accessed 2026-08-01
**Confidence**: High (vendor security advisory, the authoritative source for its own
CVE)
**Verification**: [GitHub Advisory Database — CVE-2026-24834](https://github.com/advisories/GHSA-wwj6-vghv-5p64) — Accessed 2026-08-01; [SentinelOne vulnerability database entry](https://www.sentinelone.com/vulnerability-database/cve-2026-24834/) — Accessed 2026-08-01
**Analysis (interpretation)**: This is the **freshest and most decisive finding in the
document**, and it is only six months old. The most sophisticated project in this
space tried virtio-pmem + DAX as the guest rootfs on Cloud Hypervisor, shipped it,
took a CVSS 9.3, **and the remediation was to retreat to plain `virtio-blk`**. The
lesson for Overdrive is blunt: on Cloud Hypervisor, **virtio-blk is the path that
upstream converged on after being burned by the alternative.** Do not re-derive the
pmem/DAX rootfs idea from first principles; it has been tried and withdrawn.

### 5. Other platforms

**Source-quality warning for this section.** Most of these are commercial vendors
writing about their own products. Per the bias checklist, every one has a commercial
interest in the conclusion. I have separated the ones with **real technical
substance** from the ones that are **marketing**, and flagged unmethodologised
numbers. Confidence across this section is **Medium** unless noted.

#### 5.1 Modal — the most technically substantive of the non-hyperscalers

**Evidence**: "A Modal container filesystem is an **OverlayFS** filesystem where the
read-only lower is a **FUSE-based lazy loading file server**". The image "serves as an
index; essentially, it's a data structure that holds all the files and metadata about
those files" — the index is "around **five megabytes** and can be loaded in **one to
100 milliseconds**." Reads flow through a content-addressed tiered cache. Their stated
tier latencies:

| Tier | Read latency | Throughput |
|---|---|---|
| Memory | 1–100 ns | 10–40 GiB/s |
| SSD | 100 μs | 4 GiB/s |
| AZ cache server | 1 ms | 10 GiB/s |
| Regional CDN | 100 ms | 3–10 GiB/s |
| Blob storage | 200 ms | 3–10 GiB/s |

Achieved "about **2.5 gigabytes per second**", loading "a 512 MiB `.safetensors` file
in just **200 milliseconds** from disk cache and about **300 milliseconds** from the
network." Tuning that mattered: FUSE read-ahead raised from the default **128 KB to
32 MB** ("has worked wonders"); FUSE request size from **128 KB to 1 MB** "for peak
throughput".
**Source**: [Modal — "Fast, lazy container loading in Modal"](https://modal.com/blog/jono-containers-talk) — Accessed 2026-08-01
**Confidence**: Medium-High for the architecture (detailed, self-consistent, engineer-authored talk transcript); **Medium** for the numbers — **vendor numbers with
no stated hardware, kernel, or repeatable methodology**. The tier-latency table in
particular reads as design-guidance order-of-magnitude figures, not measurements.
**Verification**: [Modal — "How Modal speeds up container launches in the cloud"](https://modal.com/blog/speeding-up-container-launches) — Accessed 2026-08-01; [machines.fail notes on the same talk](https://machines.fail/notes/fast,-lazy-container-loading-in-modal-2024) — Accessed 2026-08-01
**Analysis (interpretation)**: Two transferable insights. (1) **The image as an
index**: a ~5 MB metadata structure is the *only* thing that must be present before
start; contents stream. (2) **The FUSE tuning numbers are the most practically useful
data in this whole section** — a 250× read-ahead increase and an 8× request-size
increase were needed to make FUSE competitive. That is corroborating evidence for the
anti-FUSE finding: FUSE is workable but only after significant tuning, and Lambda
(§3.6) chose to leave rather than tune further. **Caveat**: Modal's default runtime is
gVisor, not a microVM; Modal VM Sandboxes are the microVM product. The lazy-loading
filesystem work is described in the gVisor context.

#### 5.2 E2B — squashfs read-only base + per-instance ext4 overlay

**Evidence**: E2B builds "a compressed **Squashfs** image of the root filesystem" —
"Squashfs is a compressed read-only filesystem that is very fast to load and can be
mounted as a read-only filesystem." Each instance gets "its own **ext4 writable
overlay** mounted on top of the read-only base using **OverlayFS**". Motivation stated
plainly: "copying the root filesystem for each instance is not the best idea. Even
with small root filesystems like Alpine Linux, you will probably have a few hundred
megabytes of data to copy." Instances "share a single base filesystem while
maintaining isolated writable layers through **sparse files**." Separately, E2B's
template system builds `rootfs.ext4` + `snapfile` (VM state snapshot) + `memfile`
(memory snapshot) from a Dockerfile via `e2b template build`.
**Source**: [E2B — "Scaling Firecracker: Using OverlayFS to Save Disk Space"](https://e2b.dev/blog/scaling-firecracker-using-overlayfs-to-save-disk-space) — Accessed 2026-08-01
**Confidence**: Medium-High for the architecture (concrete, mechanism-level, and
reproducible from the description); **Low** for the "<200ms sandbox initialization"
figure, which appears in third-party summaries without methodology — treat as
**UNVERIFIED**.
**Verification**: [e2b-dev/infra — Firecracker integration (DeepWiki)](https://deepwiki.com/e2b-dev/infra/3.2-firecracker-integration) — Accessed 2026-08-01 (**note**: DeepWiki is AI-generated documentation over the repo — treat as a
pointer to the source, not as an independent authority)
**Analysis (interpretation)**: **This is the closest published architecture to what
Overdrive should build.** Squashfs read-only shared base + per-VM ext4 overlay + sparse
files is exactly the "cheap writable layer over a shared read-only image" shape, it
needs no FUSE daemon, no devmapper thin-pool, and no chunk server. It is the
low-complexity end of the design space and it demonstrably works in production.

#### 5.3 Northflank — Cloud Hypervisor via Kata (relevant, but marketing-grade sourcing)

**Evidence**: "Northflank uses Kata Containers with Cloud Hypervisor as the primary
VMM for microVM isolation… Kata handles all the orchestration: provisioning the VM,
booting a minimal guest kernel, mounting your container image, managing networking."
**Source**: [Northflank — "Guide to Cloud Hypervisor in 2026"](https://northflank.com/blog/guide-to-cloud-hypervisor) — Accessed 2026-08-01
**Confidence**: **Low**. On direct examination this post contains **no measured
performance data specific to Northflank's deployment**, no rootfs-format details, and
no image-processing specifics. Generic figures quoted ("boots VMs in ~200ms",
"~50,000 lines" of Rust) are attributed to Cloud Hypervisor generally, not measured by
Northflank. Scale claims ("thousands of secure sandboxes") carry "zero metrics,
methodology, or performance data". Flagged as **vendor marketing**.
**Analysis (interpretation)**: The one genuinely useful datum: **a commercial platform
runs Cloud Hypervisor microVMs in production by delegating the whole OCI→VM problem to
Kata.** That is a legitimate "buy, don't build" data point — but Kata brings
containerd, a shim, and a Go/Rust runtime stack that Overdrive does not have and
would not want.

#### 5.4 Koyeb, Daytona, Depot, Railway, Sprites — thin or absent public detail

**Evidence and status**:

| Platform | What is publicly established | Confidence |
|---|---|---|
| **Koyeb** | Uses Firecracker for serverless multi-tenancy — "Firecracker is an open-source technology used at Koyeb to power serverless workloads". Their published posts are **explainers about Firecracker**, not descriptions of Koyeb's own image pipeline. | Low — no image-pipeline detail found |
| **Daytona** | Default sandboxes "run as **Linux containers**", with VM sandboxes as a separate option; uses Sysbox for harder container isolation. Sub-90 ms cold starts claimed via containers, **not** microVMs. | Low — and largely *not* a microVM image-factory story |
| **Sprites** | Named among Firecracker-based platforms (with Fly.io, Vercel Sandbox, E2B). No public rootfs/image-pipeline writeup found. | **UNVERIFIED** |
| **Depot** | No credible technical writeup on OCI→microVM rootfs found in this search. Depot's public engineering content centres on **build acceleration**, a different problem. | **UNVERIFIED** |
| **Railway** | No credible technical writeup on OCI→microVM rootfs found. | **UNVERIFIED** |

**Sources**: [Koyeb — "Lightweight Virtualization: the Container Ecosystem and Firecracker MicroVMs for Serverless"](https://www.koyeb.com/blog/lightweight-virtualization-the-container-ecosystem-and-firecracker-microvms-for-serverless) — Accessed 2026-08-01; [Daytona Sandboxes docs](https://www.daytona.io/docs/en/sandboxes/) — Accessed 2026-08-01
**Confidence**: Low across the row — this is a **documented negative result**, not a
finding. See Knowledge Gaps.
**Analysis (interpretation)**: The absence is itself informative: **the OCI→microVM
rootfs pipeline is treated as proprietary by most commercial sandbox vendors.** The
only genuinely detailed public accounts are Kata, firecracker-containerd, the Lambda
paper, Modal, and E2B. Overdrive should not expect to find a fifth reference design.

#### 5.5 A recurring community pattern worth naming

**Evidence**: A general pattern visible across small OSS projects: "pull a Linux
kernel from an OCI registry, convert a container image into an **immutable ext4
rootfs**, and layer a **writable data disk** on top using **overlayfs**."
**Source**: [cvhariharan/mvm — "A quick CLI to create development sandbox microVMs from container images"](https://github.com/cvhariharan/mvm) — Accessed 2026-08-01
**Confidence**: Low as an authority (small single-author project), but **useful as
corroboration** that the ext4-plus-overlay shape is what people independently converge
on.
**Verification**: [Firecracker `docs/rootfs-and-kernel-setup.md`](https://github.com/firecracker-microvm/firecracker/blob/main/docs/rootfs-and-kernel-setup.md) — Accessed 2026-08-01 — the upstream-recommended manual path is exactly
`dd` → `mkfs.ext4` → mount → populate from a container export.
**Analysis (interpretation)**: When the upstream VMM docs, a hobby CLI, and a funded
startup (E2B) all independently land on "read-only base image + overlay writable
layer", that convergence is a strong signal for the simplest viable design.

### 6. Rootfs format comparison

#### 6.1 The comparison table

Cost columns are **qualitative rankings** synthesised from the cited findings, not
measurements on a common bench — no source benchmarks all five formats together. Read
them as ordinal, not cardinal.

| Format | Build cost | Boot cost | Page-cache sharing across VMs | Writable? | Writable-layer composition |
|---|---|---|---|---|---|
| **ext4 image** (virtio-blk) | Low — `dd` + `mkfs.ext4` + populate; deterministic builder needed for dedup | Low — guest kernel mounts a block device directly, no daemon | **No** by default (each VM has its own page cache over its own device); host-side sharing only via CoW blocks | **Yes**, natively | Not needed — or host-side CoW (reflink / dm-thin) per VM |
| **squashfs** (virtio-blk, read-only) | Low–Medium — `mksquashfs`, compresses | Low — in-kernel, no daemon; compressed so less I/O, some CPU | No (in-guest); host page cache of the *backing file* is shared if VMs share the file | **No** | overlayfs in guest: squashfs lower + ext4/tmpfs upper (E2B's model, §5.2) |
| **erofs** (virtio-blk, read-only) | Low–Medium — `mkfs.erofs` | **Lowest of the compressed options** — fixed-output compression, 4 KiB physical clusters, random-access directories | **Yes, uniquely**: "page cache sharing among inodes with identical content fingerprints on the same machine"; also supports **FSDAX on uncompressed inodes** | **No** | overlayfs in guest (erofs lower + writable upper) — the nydus/Kata model (§1.7) |
| **virtiofs** (host dir) | **None** — no image build at all; hand in a host directory | Medium–High — userspace `virtiofsd` in the hot path; FUSE overhead measured at 0.78× native (§1.8) | Theoretically via DAX — **but DAX is unavailable on Cloud Hypervisor (§4.4)** | **Yes** | overlayfs on the *host*, exposed as one merged dir |
| **initramfs / CPIO** | Low — `cpio` archive | **Lowest to first instruction**, but the whole image is decompressed **into RAM** before init runs | **No** — each VM holds a private full copy in RAM | Yes (it's tmpfs) | N/A — it is entirely writable, and entirely RAM-resident |

#### 6.2 Evidence for the erofs advantages

**Evidence**: EROFS's stated design goals are "secure immutable storage… immutable and
bit-for-bit identical to the official golden image" and "minimizing storage overhead
with guaranteed end-to-end performance by using compact (meta)data layout, optimized
transparent data compression, deduplication and direct access." It supports "**Page
cache sharing among inodes with identical content fingerprints on the same machine**"
and "**Direct I/O and FSDAX support on uncompressed inodes** for use cases such as
**secure containers**, loop devices, and ramdisks." Compression is per-inode selectable
across "LZ4, MicroLZMA, DEFLATE and Zstandard". The docs name "container images" and
"secure containers" as explicit target use cases.
**Source**: [Linux kernel documentation — EROFS](https://docs.kernel.org/filesystems/erofs.html) — Accessed 2026-08-01
**Confidence**: High (kernel.org primary documentation)
**Verification**: [EROFS project — Features and Comparison](https://erofs.docs.kernel.org/en/latest/features.html) — Accessed 2026-08-01; [LWN — "An introduction to EROFS"](https://lwn.net/Articles/934047/) — Accessed 2026-08-01
**Analysis (interpretation)**: **FSDAX on uncompressed erofs is the DAX path that
actually exists.** It does not depend on virtiofsd and is therefore unaffected by §4.4.
The catch is that DAX requires the *uncompressed* layout, so you trade image size for
page-cache sharing — an explicit, tunable knob rather than a missing feature. Note also
that CVE-2026-24834 (§4.6) was about **virtio-pmem's** read-only enforcement, not about
erofs; but any DAX-over-pmem design must confront the same read-only-mapping question.

#### 6.3 erofs vs squashfs — the mechanism behind the difference

**Evidence**: "SquashFS packs data with a fixed *input* block size resulting in
variable sized compressed chunks", whereas "EROFS uses fixed *output* compression where
the compressed chunks generated are fixed in size." EROFS uses "block-sized physical
clusters by default (usually **4 KiB**)" vs SquashFS's **128 KiB**. Consequence: EROFS
"delivers 3x the random read IOPS because it decompresses at page granularity instead
of block granularity". Also "SquashFS does not allow random-access in its directories,
unlike EROFS; that means SquashFS requires linear searches for directory entries."
**Source**: [EROFS FAQ (erofs.docs.kernel.org)](https://erofs.docs.kernel.org/en/latest/faq.html) — Accessed 2026-08-01
**Confidence**: Medium-High. The *mechanism* (fixed-output vs fixed-input, 4 KiB vs
128 KiB clusters, directory random access) is High — it is the project's own
documentation of its own format. The **"3× random read IOPS"** figure is a
project-authored claim about a competitor and is **not independently verified here** —
treat as **vendor-adjacent**.
**Verification**: [sigma-star — "EROFS vs. SquashFS: A Gentle Benchmark"](https://sigma-star.at/blog/2022/07/squashfs-erofs/) — Accessed 2026-08-01 (independent third party, but dated 2022); [linux-erofs mailing list — "Worse performance than SquashFS for small filesystems"](https://lists.ozlabs.org/pipermail/linux-erofs/2022-May/006417.html) — Accessed 2026-08-01
**Analysis (interpretation)**: Note the **counter-evidence** in that last citation: an
upstream mailing-list thread titled "Worse performance than SquashFS for small
filesystems". EROFS is not universally better — its advantages concentrate in
**random-read-heavy** and **large-image** cases. For a small image read mostly
sequentially at boot, squashfs is competitive and has a decade more deployment history.

#### 6.4 Why initramfs is a trap for workload rootfs

**Evidence**: An initrd is "a compressed `cpio(1)` archive… loaded into memory and used
as part of the Linux startup process. During startup, the kernel unpacks it into a
special instance of a **tmpfs** mount that becomes the initial root filesystem." Kata's
size comparison: initrd "10MB+" vs rootfs image "100MB+".
**Source**: [kata-containers `docs/design/architecture/guest-assets.md`](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/guest-assets.md) — Accessed 2026-08-01
**Confidence**: High
**Analysis (interpretation)**: initramfs is excellent for a **10 MB guest agent image**
and terrible for a **500 MB workload image** — the entire archive is decompressed into
RAM, so a 500 MB image costs 500 MB of guest RAM before the workload allocates a byte,
with **zero sharing across VMs**. The correct use is Kata's: initramfs for the *guest
system*, a separate device for the *workload rootfs*.

### 7. Layer caching and cold start — what actually wins

The question "how do you avoid re-materialising a rootfs per launch?" has four
distinct published answers. They are **not** mutually exclusive, and they sit at
different points on a complexity curve.

| Approach | Who uses it | What it costs | What it buys | Verdict for a single-node appliance |
|---|---|---|---|---|
| **Shared read-only base + per-VM overlay** | E2B (§5.2), Kata+nydus in-guest (§1.7) | Near-zero — one `mksquashfs`/`mkfs.erofs` per image; a sparse file per VM | Materialise the image **once per node**, not once per launch | **Wins on effort/benefit.** No daemon, no thin-pool, no chunk server |
| **Host-side CoW block snapshots** (dm-thin) | firecracker-containerd, containerd devmapper (§2.2) | Dedicated LVM thin-pool provisioned at install; `dmsetup` dependency; loopback config explicitly "**slow and not recommended for production**" (§2.3) | Writable rootfs per VM at block level, CoW-cheap | Real cost: it makes the **node disk layout** part of the contract |
| **Content-addressed chunk store + demand loading** | Lambda (§3.1–3.6), Modal (§5.1), nydus (§1.7) | Highest — deterministic image builder, chunk store, manifest format, a demand-load path (FUSE/userfaultfd), a cache | Only fetch what is read (**"76% of startup is pull, but only 6.4% of that data is read"**, §1.7); dedup across images | **Right end-state, wrong first slice.** 2/3 of Lambda's hits are the *local* cache (§3.4) — which a single node gets from a plain local store |
| **Full VM snapshot / restore** | E2B (`snapfile`+`memfile`, §5.2), Modal memory snapshots | Snapshot format, memory file management, and it **sidesteps rather than solves** the rootfs problem | Sub-second start by skipping boot entirely | Orthogonal — a later optimisation *on top of* whichever rootfs format you pick |

#### 7.1 The decisive evidence: locality dominates

**Evidence**: Lambda's measured tier distribution — "median of **67% of chunks were
loaded from the on-worker cache**, 32% from the AZ-level distributed cache, and the
remaining **0.06% from the backing store**" — and the latency gap that motivates it:
local-cache hit "mode below 100 μs" vs L2 hit "mode around 2.75 ms" vs S3 origin
"36 ms" median.
**Source**: [arXiv:2305.13162](https://ar5iv.labs.arxiv.org/html/2305.13162) — Accessed 2026-08-01
**Confidence**: High
**Analysis (interpretation)**: **Two-thirds of the benefit of the entire elaborate
Lambda pipeline is available from a plain on-node cache.** The distributed tier exists
because Lambda has thousands of workers and cannot pin an image to one of them; a
single-node appliance has that property for free. This is the strongest argument in
the document for Overdrive deferring chunk-level machinery.

#### 7.2 Reflink/CoW on the host (XFS/btrfs) — a documented gap

**Evidence**: I searched for measured comparisons of host-filesystem reflink
(`FICLONE` on XFS/btrfs) as the CoW mechanism for per-VM rootfs copies versus dm-thin
and versus overlayfs, and **found no authoritative measured source**. E2B's use of
"**sparse files**" for per-instance overlays (§5.2) is the closest published practice.
**Confidence**: **Low — this is a knowledge gap, not a finding.** See Knowledge Gaps.
**Analysis (interpretation)**: Reflink is *mechanically* attractive — `cp --reflink=always`
of a base ext4 image is O(1) and gives a fully writable, independent rootfs per VM with
no overlayfs, no thin-pool, and no daemon. That it is not prominent in the published
literature may reflect that the big players are multi-tenant (and need dedup across
tenants, which reflink does not give) rather than that it does not work. **Overdrive
should measure this itself rather than infer it.**

### 8. The in-guest agent question

#### 8.1 vsock is universal — confirmed

**Evidence**: Kata: the runtime communicates "with the agent using a **ttRPC based
protocol over a VSOCK socket**" (§1.5). firecracker-containerd: the in-VM agent acts
"on control instructions received from the runtime", with the shim "communicating over
ttrpc" (§2.1). Cloud Hypervisor ships **virtio-vsock**: "a hybrid implementation of the
VSOCK socket address family over virtio" for "efficient and secure host-guest
communication", and it is hot-pluggable (`add-vsock cid=…`).
**Source**: [Kata Architecture](https://kata-containers.github.io/kata-containers/design/architecture/), [firecracker-containerd `docs/architecture.md`](https://github.com/firecracker-microvm/firecracker-containerd/blob/main/docs/architecture.md), [cloud-hypervisor `docs/device_model.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/device_model.md) — all Accessed 2026-08-01
**Confidence**: **High** — three independent implementations, all primary sources,
all vsock. The question "is vsock near-universal?" is **confirmed**.
**Analysis (interpretation)**: vsock is the right transport for the obvious reason: it
needs no IP addressing, no routing, no netns plumbing, and it works before (and
independently of) guest networking. That last property matters a lot to Overdrive,
whose networking model is per-workload netns/veth — the agent channel must not depend
on the very thing the agent may be helping configure.

#### 8.2 What the agent minimally does

**Evidence**, synthesised from the two published agents:

| Responsibility | Kata `kata-agent` | firecracker-containerd agent |
|---|---|---|
| Exec the entrypoint | Yes — "agent creates the user's container environment" | Yes — "invokes runC through containerd's shim to create standard Linux containers" |
| Report exit code / lifecycle | Yes — ttRPC API incl. `DestroySandbox` | Yes — "emitting event and metric information to the runtime" |
| Stream logs / stdio | Yes | Yes — "proxying STDIO for container processes" |
| Mount volumes / rootfs | Yes — mounts virtio-fs/RAFS at container paths (§1.7) | Yes — mounts the block device |
| Configure network | Yes (agent-side interface config) | Yes |
| Be PID 1 | In the **initrd** shape, yes (§1.1) | No — runs under the guest's init |

**Source**: [kata-containers guest-assets](https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/guest-assets.md), [Kata Architecture](https://kata-containers.github.io/kata-containers/design/architecture/), [firecracker-containerd `docs/architecture.md`](https://github.com/firecracker-microvm/firecracker-containerd/blob/main/docs/architecture.md) — Accessed 2026-08-01
**Confidence**: High for each individual row (each is a direct quote from a primary
source); **Medium** for the table as a "minimal set" — that framing is my synthesis,
not a claim any single source makes.
**Analysis (interpretation)**: The irreducible core is **three things**: exec the
entrypoint with the right env/cwd/uid, surface the exit code, and pipe stdio. Mounting
and networking can be pushed out of the agent if the host pre-composes the rootfs
(§1.2) and pre-configures the interface.

#### 8.3 Is an agentless design viable?

**Evidence**: There is no published production OCI→microVM platform without a guest
agent among those surveyed — Kata, firecracker-containerd, Lambda, E2B, and Modal all
have one. However, the *mechanism* by which an agent could be avoided is documented:
the guest kernel can be told what to run via **kernel cmdline `init=`**, and the
workload rootfs can be a complete filesystem the kernel mounts directly (Firecracker's
`rootfs-and-kernel-setup.md` boots an ext4 with a standard init; CH's `virtiofs-root.md`
passes `root=` on the cmdline, §4.2).
**Confidence**: **Medium.** The "no published agentless platform" claim is a
**negative result from my search**, not an exhaustive proof — see Knowledge Gaps.
**Analysis (interpretation)** — what agentless costs you, concretely:

1. **Exit codes.** A process exiting inside the guest is invisible to the host. Without
   an agent you must infer completion from VM shutdown, which conflates "workload
   exited 0", "workload exited 7", and "kernel panic". **This is the single biggest
   loss** and it directly breaks Overdrive's existing restart/backoff model, which is
   driven by observed exit status.
2. **stdio/logs.** Recoverable without an agent — `virtio-console` gives you a serial
   console the host can capture. Lower fidelity (no stdout/stderr separation, no
   structured framing) but real.
3. **Liveness/health.** No in-guest probe execution; health must be inferred externally
   (network probes), which is weaker than Overdrive's existing probe model.
4. **Dynamic config.** No way to push volume mounts, secrets, or network changes after
   boot without rebooting.

A **middle path** exists and is worth naming: a **~200-line static init binary** that is
PID 1, execs the entrypoint, and writes exit status to a vsock socket. That is not
"no agent" but it is dramatically less than `kata-agent` (which implements a full
container runtime API). It buys back items 1 and 2 for a very small cost.

#### 8.4 The agent is where the security boundary bites

**Evidence**: `virtiofsd` "uses a hardened FUSE implementation that **does not trust the
client**, which is important because in virtiofs the client is the **untrusted VM** and
the file system daemon must not trust it."
**Source**: [virtio-fs project site](https://virtio-fs.gitlab.io/) — Accessed 2026-08-01
**Confidence**: Medium-High (project's own documentation)
**Verification**: [Cloud Hypervisor `docs/fs.md`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/fs.md) — Accessed 2026-08-01
**Analysis (interpretation)**: Every host-side daemon that parses guest-controlled input
(virtiofsd, a FUSE chunk server, an agent-facing RPC endpoint) is attack surface. A
**block device is the narrowest possible interface** — the host hands over bytes and
parses nothing from the guest. This is a real, if secondary, argument for virtio-blk on
top of the performance argument. For a **single-tenant appliance** the threat model is
softer than Kata's, but "softer" is not "absent": the workload is still less trusted
than the node.

## Source Analysis

| Source | Domain | Reputation | Type | Access date | Cross-verified |
|---|---|---|---|---|---|
| Brooker et al., ATC '23 (arXiv:2305.13162) | arxiv.org / usenix.org | High (1.0) | Academic, peer-reviewed | 2026-08-01 | Y |
| Linux kernel docs — EROFS | docs.kernel.org | High (1.0) | Official | 2026-08-01 | Y |
| Kata `guest-assets.md` | github.com/kata-containers | High (1.0) | Official project design doc | 2026-08-01 | Y |
| Kata Architecture | kata-containers.github.io | High (1.0) | Official | 2026-08-01 | Y |
| Kata Storage design | kata-containers.github.io | High (1.0) | Official | 2026-08-01 | Y |
| Kata `kata-nydus-design.md` | github.com/kata-containers | High (1.0) | Official | 2026-08-01 | Y |
| Kata advisory GHSA-wwj6-vghv-5p64 (CVE-2026-24834) | github.com/kata-containers | High (1.0) | Official security advisory | 2026-08-01 | Y |
| Cloud Hypervisor `fs.md` | github.com/cloud-hypervisor | High (1.0) | Official | 2026-08-01 | Y |
| Cloud Hypervisor `device_model.md` | github.com/cloud-hypervisor | High (1.0) | Official | 2026-08-01 | Y |
| Cloud Hypervisor `hotplug.md` | github.com/cloud-hypervisor | High (1.0) | Official | 2026-08-01 | Y |
| Cloud Hypervisor `virtiofs-root.md` | github.com/cloud-hypervisor | High (1.0) | Official | 2026-08-01 | N (single) |
| containerd `devmapper.md` | github.com/containerd | High (1.0) | Official (CNCF) | 2026-08-01 | Y |
| firecracker-containerd `architecture.md` / `design-approaches.md` / commits | github.com/firecracker-microvm | High (1.0) | Official | 2026-08-01 | Y |
| Firecracker `rootfs-and-kernel-setup.md` | github.com/firecracker-microvm | High (1.0) | Official | 2026-08-01 | Y |
| EROFS FAQ / Features | erofs.docs.kernel.org | High (1.0) | Official | 2026-08-01 | Partial (self-authored comparison) |
| LWN — "An introduction to EROFS" | lwn.net | High (1.0) | Technical journalism | 2026-08-01 | Y |
| virtio-fs project site + design | virtio-fs.gitlab.io | High (1.0) | Official | 2026-08-01 | Y |
| CNCF — Evolution of Nydus | cncf.io | Medium-High (0.8) | Foundation blog, **vendor-authored** | 2026-08-01 | Partial |
| Red Hat Bugzilla 1890692 | bugzilla.redhat.com | Medium-High (0.8) | Vendor issue tracker | 2026-08-01 | Y |
| Modal engineering blog | modal.com | Medium (0.6) | **Vendor**, engineer-authored | 2026-08-01 | Partial |
| E2B engineering blog | e2b.dev | Medium (0.6) | **Vendor**, technical | 2026-08-01 | Partial |
| virtio-fs/qemu issue #20; kata runtime #2138 | gitlab.com / github.com | Medium (0.6) | Issue-tracker discussion | 2026-08-01 | Y |
| sigma-star EROFS-vs-SquashFS benchmark | sigma-star.at | Medium (0.6) | Independent, **dated 2022** | 2026-08-01 | N |
| Kata Containers Medium posts (1.7 highlights, 2.0 tuning) | medium.com | Medium (0.6) | Project-adjacent | 2026-08-01 | Y |
| Northflank blog | northflank.com | Medium (0.6) | **Vendor marketing** — flagged | 2026-08-01 | N |
| Koyeb blog | koyeb.com | Medium (0.6) | **Vendor** | 2026-08-01 | N |
| StackHPC Kata I/O performance PDF | stackhpc.com | Medium (0.6) | Industry, **undated in fetch** | 2026-08-01 | N |
| DeepWiki (e2b, kata, fc-containerd) | deepwiki.com | Low — **AI-generated** | Derived | 2026-08-01 | Used only as pointer |
| cvhariharan/mvm | github.com | Low (0.4) | Hobby OSS | 2026-08-01 | Corroboration only |

**Reputation distribution**: High: 18 (≈60%) | Medium-High: 2 (≈7%) | Medium: 9 (≈30%)
| Low/pointer-only: 2 (≈7%). **Average ≈ 0.85.**

**Bias note**: §5 is dominated by vendors describing their own products; §1.8's
headline numbers are authored by the Nydus/Alibaba team. Both are flagged inline.
The load-bearing conclusions (§3, §4, §6) rest on peer-reviewed, kernel.org, and
project-official sources.

## Knowledge Gaps

### Gap 1: No common-bench comparison of the five rootfs formats
**Issue**: No source benchmarks ext4 / squashfs / erofs / virtiofs / initramfs on one
harness for microVM boot. §6.1's cost columns are ordinal synthesis, not measurement.
**Attempted**: erofs-vs-squashfs searches, Kata performance posts, StackHPC I/O paper.
**Recommendation**: Overdrive should measure this itself on the pinned 6.18 kernel —
it is a half-day Tier-3 experiment and the answer is node-specific anyway.

### Gap 2: Host reflink (`FICLONE` on XFS/btrfs) as the per-VM CoW mechanism
**Issue**: No authoritative measured comparison of reflink vs dm-thin vs overlayfs for
per-VM rootfs materialisation (see §7.2). This is arguably *the* most promising
low-complexity option for a single-tenant node and it is the least documented.
**Attempted**: caching/cold-start searches; E2B's "sparse files" is the nearest
published practice.
**Recommendation**: Measure directly. Likely a strong result for Overdrive precisely
because the published players are multi-tenant and reflink does not serve them.

### Gap 3: virtiofsd DAX status is sourced to an undated discussion
**Issue**: CH's "not available in Cloud Hypervisor" is authoritative and current, but
the *reason* (Rust virtiofsd lacks DAX; vhost-user commands never went upstream) comes
from a Red Hat RFE and forum threads without a clear 2025–2026 dateline.
**Attempted**: Targeted 2025/2026 status searches; no dated release note found.
**Recommendation**: Re-check `virtiofsd` release notes before relying on the *absence*
persisting. The operational conclusion (don't design for virtiofs DAX on CH today) is
unaffected.

### Gap 4: Sprites, Depot, Railway have no public OCI→microVM writeups
**Issue**: Marked **UNVERIFIED** in §5.4. Requested by name; no credible technical
sources found.
**Attempted**: Multiple targeted searches per vendor.
**Recommendation**: Treat as unavailable. Overdrive has a prior internal note at
`docs/research/platform/sprites-as-overdrive-primitive-research.md` that may carry more.

### Gap 5: No exhaustive proof that agentless designs are unpublished
**Issue**: §8.3's "no published agentless platform" is a negative search result.
**Recommendation**: Low value to close; the cost analysis in §8.3 stands independently.

### Gap 6: Cold-start latency budgets are not comparable across sources
**Issue**: Boot-time claims (Firecracker "125ms", CH "~200ms", E2B "<200ms") come from
different sources with different definitions of "boot" and no shared methodology. All
are flagged; none are used in the recommendation.

## Conflicting Information

### Conflict 1: Is DAX enabled by default in Cloud Hypervisor?
**Position A — DAX is enabled by default with an 8 GiB cache window.** Source:
[Cloud Hypervisor docs HTML mirror](https://intelkevinputnam.github.io/cloud-hypervisor-docs-HTML/docs/fs.html), reputation Medium (0.6) — a personal GitHub Pages mirror of CH docs.
Evidence: "By default, DAX is enabled with a cache window of 8GiB."
**Position B — DAX is not available in Cloud Hypervisor at all.** Source:
[cloud-hypervisor `docs/fs.md` on `main`](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/fs.md), reputation High (1.0) — the project's own current source tree.
Evidence: "Given the DAX feature is not stable yet from a daemon standpoint, it is not
available in Cloud Hypervisor."
**Assessment**: **Position B is correct.** Position A is a **stale mirror** describing
CH's earlier in-tree `virtio-fs` implementation, before CH moved to external
`virtiofsd`. The upstream `main` doc is both more authoritative and more current, and
is independently corroborated by the Rust-virtiofsd DAX gap (§4.4). **This conflict is
a live trap** — the stale mirror ranks well in search results, and a designer who reads
it will budget for page-cache sharing that does not exist.

### Conflict 2: Is virtio-fs faster than 9pfs / than block devices?
**Position A — virtio-fs is the performance win.** Sources: Kata 2.0 tuning posts, the
virtio-fs project. Evidence: local filesystem semantics, DAX page-cache sharing.
**Position B — virtio-fs underperforms block storage for rootfs-shaped I/O, and early
builds underperformed even 9p.** Sources: [virtio-fs/qemu issue #20](https://gitlab.com/virtio-fs/qemu/-/work_items/20), [kata-containers/runtime #2138](https://github.com/kata-containers/runtime/issues/2138).
**Assessment**: **Both are true of different things, and the reconciliation is the
finding.** virtio-fs beats 9pfs on POSIX semantics and general throughput — that is why
it replaced it. It does *not* beat a block device for a container rootfs, which is why
Kata retains `disable_block_device_use=false`. The DAX claim in Position A is
**conditional on DAX existing**, which on Cloud Hypervisor it does not (Conflict 1).
Net: on CH, Position B governs.

### Conflict 3: erofs vs squashfs
**Position A**: erofs is substantially faster (claimed 3× random-read IOPS), smaller,
and better for container images. Source: [EROFS FAQ](https://erofs.docs.kernel.org/en/latest/faq.html) — High reputation as documentation, but **self-authored comparison against a competitor**.
**Position B**: erofs can be *worse* than SquashFS for small filesystems. Source:
[linux-erofs mailing list](https://lists.ozlabs.org/pipermail/linux-erofs/2022-May/006417.html) — upstream discussion on the EROFS project's own list.
**Assessment**: Position A is right for **large, random-read-heavy** images; Position B
warns that **small, sequentially-read** images may not benefit. Both come from the EROFS
project's own orbit, which is a **circular-reference risk** — I found no strong recent
independent benchmark (the sigma-star one is from 2022). Treat the erofs advantage as
**real but workload-dependent and not independently confirmed at 2026 kernels**.

## Recommendation for Overdrive

Framing constraints that change the answer versus every system surveyed: **single
tenant** (no cross-tenant dedup requirement, no untrusted-neighbour threat model),
**pinned kernel** (you control guest kernel config — no "will the guest support this?"
question), **immutable appliance OS** (host disk layout is a build-time decision),
**existing per-workload netns/veth/mTLS** (networking is already solved outside the VM),
and **zero existing image machinery** (nothing to be backward-compatible with).

### The thinnest first slice

**Format: a single `ext4` image per workload image, attached as `virtio-blk`, built by
flattening the OCI image.**

This is the Lambda flattening step (§3.1) minus everything else, and it is what
Firecracker's own docs describe as the manual path (§5.5). Concretely, slice 1 is:

1. **Pull** — a minimal OCI distribution client (registry auth, manifest, layer blobs)
   writing blobs into a content-addressed directory keyed by digest. No new storage
   engine; the digest *is* the key, and OCI already gives it to you.
2. **Flatten** — untar layers in order, applying whiteouts, onto a fresh `ext4` image
   sized from the manifest. Cache the result keyed by the image's **config digest** so
   the flatten happens **once per image, not once per launch**.
3. **Boot** — `virtio-blk` with the flattened image, `root=/dev/vda` on the kernel
   cmdline, plus the pinned kernel. Cloud Hypervisor's documented boot path (§4.1).
4. **Per-launch writable layer** — `cp --reflink=auto` of the cached ext4 image (O(1) on
   XFS/btrfs; falls back to a real copy elsewhere, which is correct-but-slow rather than
   broken). Measure this against a squashfs+overlay variant (Gap 2).

**Why not the alternatives, for slice 1:**

| Rejected for slice 1 | Because |
|---|---|
| virtio-fs / virtiofsd | DAX unavailable on CH (§4.4) so the main benefit evaporates; adds a supervised host daemon per VM; requires `--memory shared=on`; FUSE measured 0.78× native (§1.8) and Lambda is *leaving* FUSE (§3.6); CH's own virtiofs-root doc says not production-ready (§4.2) |
| virtio-pmem + DAX | **CVE-2026-24834, CVSS 9.3** — Kata shipped exactly this on CH and the fix was to retreat to virtio-blk (§4.6) |
| devmapper thin-pool | Forces an LVM thin-pool into the appliance disk layout; loopback mode is "not recommended for production" (§2.3); solves a CoW problem reflink may solve for free |
| Chunked content-addressed store | 2/3 of Lambda's benefit is the *local* cache (§3.4), which a single node gets from step 1's blob directory. Real, but it is the *end state*, not the first slice |
| initramfs for the workload | Whole image into guest RAM, zero cross-VM sharing (§6.4) |
| squashfs/erofs in slice 1 | Both are good (E2B ships squashfs, §5.2) but each **requires** an in-guest overlay to get writability. ext4+reflink gets a writable rootfs with no overlay at all. Revisit at step 2 |

**Guest agent in slice 1: yes — but the ~200-line version, not `kata-agent`.**

A static PID-1 init that (a) execs the entrypoint with env/cwd/uid from the OCI config,
(b) writes the exit status to **vsock**, and (c) forwards stdio. Not the full Kata
agent. The justification is specific rather than cargo-culted: **Overdrive's existing
restart/backoff model is driven by observed exit status** (`.claude/rules` reconciler
retry memory persists `attempts` + `last_failure_seen_at` from *observed* failures). An
agentless VM cannot report an exit code (§8.3 item 1), so agentless would silently
degrade a control-plane behaviour that already exists and is already tested. That is a
worse trade than writing 200 lines of Rust. vsock is confirmed universal (§8.1) and CH
supports it natively and hot-pluggably.

**Explicitly deferred from slice 1** (and these are *not* deferrals needing issues —
they are simply out of scope until the loop closes): chunk-level dedup, lazy/demand
loading, convergent encryption (**never needed** — single tenant, §3.1), erasure
coding, VM snapshot/restore, in-guest overlayfs, registry mirroring.

**The vertical slice must close through `overdrive serve` + `overdrive deploy`.** Per
the project's own rule, the bar is a real deploy of a spec naming an OCI image that
boots in CH and reports its exit code — not a test harness that assembles the pieces.
If that is too large for one step, the correct reduction is a **narrower image**
(a pinned known-good image, no registry auth) rather than a deferred production path.

### The right end-state

1. **Format**: **erofs** (uncompressed or lightly compressed) as the read-only base +
   in-guest **overlayfs** writable upper. erofs is the only surveyed format with
   **page-cache sharing across identical content on one machine** and **FSDAX on
   uncompressed inodes** (§6.2) — precisely the cross-VM memory sharing that virtiofs
   DAX was supposed to deliver and cannot on CH. On a node running many VMs off a few
   base images, this is the memory-density win. Gate the move on measuring Gap 1.
2. **Caching**: content-addressed **local** blob + built-image store, keyed by OCI
   digests, with a flattened-image cache keyed by config digest. Add chunking **only**
   if measurements show materialisation time dominating — and note Lambda's own data
   says the local tier is where the wins are (§3.4).
3. **Agent**: grow the slice-1 init into a small ttRPC-over-vsock agent as (and only as)
   volumes, exec-into-running-workload, and in-guest probes are needed. Follow Kata's
   transport choice; do **not** follow its API surface, which is sized for a
   general-purpose container runtime Overdrive does not need.
4. **Never adopt**: convergent encryption (single tenant), erasure coding (single node),
   containerd/snapshotter integration (no containerd), virtio-pmem DAX rootfs (§4.6).

### The one thing most likely to go wrong

Reaching for virtio-fs because "Kata defaults to it". Kata's default is for
**flexibility and POSIX compatibility in a multi-tenant containerd world**, its
performance rationale **depends on DAX**, and **DAX does not exist on Cloud
Hypervisor**. On CH the evidence — CH's own docs (§4.1), Lambda's architecture (§3.6),
the nydus FUSE numbers (§1.8), and a CVSS 9.3 retreat *to* virtio-blk (§4.6) — points
one way: **the guest should see a block device.**

## Recommendations for Further Research

1. **Measure Gap 1 and Gap 2 on the pinned 6.18 kernel** — ext4+reflink vs
   squashfs+overlay vs erofs+overlay, on boot latency, per-VM materialisation time, and
   host page-cache residency across N concurrent VMs. This is the only open question
   whose answer actually changes the slice-1 format choice, and it is cheap.
2. **Verify erofs page-cache sharing empirically** — the kernel docs claim it; confirm
   it materialises for VM-backing files, since it is the load-bearing reason to prefer
   erofs at the end state.
3. **Re-check virtiofsd DAX before any future virtio-fs decision** (Gap 3).
4. **Review `docs/research/platform/sprites-as-overdrive-primitive-research.md`** for
   the Sprites detail this survey could not source publicly.

## Full Citations

[1] Brooker, M., et al. "On-demand Container Loading in AWS Lambda". USENIX ATC '23. 2023. https://www.usenix.org/conference/atc23/presentation/brooker (arXiv: https://arxiv.org/pdf/2305.13162 ; HTML: https://ar5iv.labs.arxiv.org/html/2305.13162 ). Accessed 2026-08-01.
[2] Linux Kernel Documentation. "EROFS - Enhanced Read-Only File System". https://docs.kernel.org/filesystems/erofs.html. Accessed 2026-08-01.
[3] Kata Containers Project. "Guest Assets". https://github.com/kata-containers/kata-containers/blob/main/docs/design/architecture/guest-assets.md. Accessed 2026-08-01.
[4] Kata Containers Project. "Kata Containers Architecture". https://kata-containers.github.io/kata-containers/design/architecture/. Accessed 2026-08-01.
[5] Kata Containers Project. "Storage". https://kata-containers.github.io/kata-containers/design/architecture/storage/. Accessed 2026-08-01.
[6] Kata Containers Project. "Kata Nydus Design". https://github.com/kata-containers/kata-containers/blob/main/docs/design/kata-nydus-design.md. Accessed 2026-08-01.
[7] Kata Containers Project. "Kata Container to Guest micro VM privilege escalation" (CVE-2026-24834, GHSA-wwj6-vghv-5p64). 2026-02-19. https://github.com/kata-containers/kata-containers/security/advisories/GHSA-wwj6-vghv-5p64. Accessed 2026-08-01.
[8] Cloud Hypervisor Project. "How to use virtio-fs". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/fs.md. Accessed 2026-08-01.
[9] Cloud Hypervisor Project. "Device Model". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/device_model.md. Accessed 2026-08-01.
[10] Cloud Hypervisor Project. "Hotplug". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/hotplug.md. Accessed 2026-08-01.
[11] Cloud Hypervisor Project. "VirtioFS as root filesystem". https://github.com/cloud-hypervisor/cloud-hypervisor/blob/main/docs/virtiofs-root.md. Accessed 2026-08-01.
[12] containerd Project. "Devmapper snapshotter". https://github.com/containerd/containerd/blob/main/docs/snapshotters/devmapper.md. Accessed 2026-08-01.
[13] firecracker-containerd Project. "Architecture". https://github.com/firecracker-microvm/firecracker-containerd/blob/main/docs/architecture.md. Accessed 2026-08-01.
[14] firecracker-containerd Project. "Design Approaches". https://github.com/firecracker-microvm/firecracker-containerd/blob/main/docs/design-approaches.md. Accessed 2026-08-01.
[15] firecracker-containerd Project. Commit history, `main` branch. https://github.com/firecracker-microvm/firecracker-containerd/commits/main. Accessed 2026-08-01.
[16] Firecracker Project. "Creating Custom rootfs and kernel Images". https://github.com/firecracker-microvm/firecracker/blob/main/docs/rootfs-and-kernel-setup.md. Accessed 2026-08-01.
[17] EROFS Project. "Frequently Asked Questions". https://erofs.docs.kernel.org/en/latest/faq.html. Accessed 2026-08-01.
[18] EROFS Project. "Features and Comparison". https://erofs.docs.kernel.org/en/latest/features.html. Accessed 2026-08-01.
[19] Corbet, J. (LWN). "An introduction to EROFS". https://lwn.net/Articles/934047/. Accessed 2026-08-01.
[20] virtio-fs Project. "virtiofs - shared file system for virtual machines". https://virtio-fs.gitlab.io/. Accessed 2026-08-01.
[21] Nydus Team. "The evolution of the Nydus Image Acceleration". CNCF Blog. 2022-11-15. https://www.cncf.io/blog/2022/11/15/the-evolution-of-the-nydus-image-acceleration/. Accessed 2026-08-01.
[22] Red Hat Bugzilla. "RFE: [virtiofsd] Support DAX for faster speed" (Bug 1890692). https://bugzilla.redhat.com/show_bug.cgi?id=1890692. Accessed 2026-08-01.
[23] Modal. "Fast, lazy container loading in Modal". https://modal.com/blog/jono-containers-talk. Accessed 2026-08-01.
[24] Modal. "How Modal speeds up container launches in the cloud". https://modal.com/blog/speeding-up-container-launches. Accessed 2026-08-01.
[25] E2B. "Scaling Firecracker: Using OverlayFS to Save Disk Space". https://e2b.dev/blog/scaling-firecracker-using-overlayfs-to-save-disk-space. Accessed 2026-08-01.
[26] virtio-fs / QEMU. "Performance of virtio-fs vs. block storage for kata dind use-case" (issue #20). https://gitlab.com/virtio-fs/qemu/-/work_items/20. Accessed 2026-08-01.
[27] Kata Containers. "Poor qemu-virtiofs performance in benchmarks" (runtime issue #2138). https://github.com/kata-containers/runtime/issues/2138. Accessed 2026-08-01.
[28] Kata Containers. "clh: Enable disk block device hotplug support" (runtime PR #2681). https://github.com/kata-containers/runtime/pull/2681. Accessed 2026-08-01.
[29] linux-erofs mailing list. "Worse performance than SquashFS for small filesystems". 2022-05. https://lists.ozlabs.org/pipermail/linux-erofs/2022-May/006417.html. Accessed 2026-08-01.
[30] sigma-star. "EROFS vs. SquashFS: A Gentle Benchmark". 2022-07. https://sigma-star.at/blog/2022/07/squashfs-erofs/. Accessed 2026-08-01.
[31] Gregory, A. "Kata Containers 1.7.0 Release Highlights". Medium. https://medium.com/kata-containers/kata-containers-1-7-0-release-highlights-9e07ddbe737e. Accessed 2026-08-01.
[32] Lining2020. "Exploration and Practice of Performance Tuning for Kata Containers 2.0". Medium. https://medium.com/kata-containers/exploration-and-practice-of-performance-tuning-for-kata-containers-2-0-85055d29e8b5. Accessed 2026-08-01.
[33] Kata Containers. "How to use virtio-fs with Kata". https://github.com/kata-containers/documentation/blob/master/how-to/how-to-use-virtio-fs-with-kata.md. Accessed 2026-08-01.
[34] containerd. "nydus-snapshotter". https://github.com/containerd/nydus-snapshotter. Accessed 2026-08-01.
[35] Nydus Project. https://nydus.dev/. Accessed 2026-08-01.
[36] Northflank. "Guide to Cloud Hypervisor in 2026". https://northflank.com/blog/guide-to-cloud-hypervisor. Accessed 2026-08-01. [Vendor marketing — flagged]
[37] Koyeb. "Lightweight Virtualization: the Container Ecosystem and Firecracker MicroVMs for Serverless". https://www.koyeb.com/blog/lightweight-virtualization-the-container-ecosystem-and-firecracker-microvms-for-serverless. Accessed 2026-08-01. [Vendor]
[38] Daytona. "Sandboxes". https://www.daytona.io/docs/en/sandboxes/. Accessed 2026-08-01.
[39] Kunwar, B. (StackHPC). "Disk I/O Performance of Kata Containers". https://www.stackhpc.com/images/IO-Performance-of-Kata-Containers-TheNewStack.pdf. Accessed 2026-08-01.
[40] cvhariharan. "mvm — A quick CLI to create development sandbox microVMs from container images". https://github.com/cvhariharan/mvm. Accessed 2026-08-01.
[41] GitHub Advisory Database. "CVE-2026-24834". https://github.com/advisories/GHSA-wwj6-vghv-5p64. Accessed 2026-08-01.

## Research Metadata

**Duration**: ~45 turns | **Sources examined**: ~45 | **Sources cited**: 41 |
**Cross-referenced findings**: 24 of 30 | **Confidence distribution**: High ≈ 60%,
Medium ≈ 33%, Low ≈ 7% | **Output**:
`docs/research/platform/oci-image-to-microvm-rootfs-research.md`

**Tool failures affecting coverage**: `usenix.org` returned HTTP 403 to automated fetch
(mitigated via the arXiv/ar5iv HTML of the same paper); `alibabacloud.com` returned
empty content (mitigated via the CNCF version of the same material — note these two are
**not independent**, same authors).
