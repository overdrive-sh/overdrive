# Research: How Fly.io Implements microVMs, End to End

**Date**: 2026-08-01 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High | **Sources**: 30

**Consumer**: Overdrive — Rust workload orchestrator building a Cloud Hypervisor microVM driver.
Existing: `Driver` port trait, exec driver, per-workload netns + veth + nft-TPROXY transparent mTLS,
cgroup v2. Missing: **all** image/rootfs machinery.

## Executive Summary

Fly.io's microVM stack is best understood as **four independent mechanisms bolted to a per-host
supervisor**: (1) Firecracker as the VMM, chosen over Xen for its virtio-only device set and ~125 ms
boot; (2) an OCI pull that lands container layers as **block devices** on an LVM2 thin pool, giving
copy-on-write per-machine rootfs snapshots; (3) a platform-owned Rust `init` that rides on its **own
block device** (`/dev/vda`, 64 MB ext2) rather than being injected into the workload image, reading
its configuration from a JSON file and talking to the host over vsock; and (4) `flyd`, a per-worker
Go daemon that models every Machine operation as a **durable finite state machine journaled to an
append-only BoltDB log**, so a crashed supervisor resumes mid-operation rather than restarting. There
is no global consensus over placement — `flaps` collects capacity from regional `flyd`s and ranks
them, market-style, so a cold start can be served **synchronously** by a specific host.

The most transferable ideas for Overdrive are the init-on-its-own-device pattern (I-1) and the
FSM-with-a-journal supervisor model (I-2), which maps cleanly onto Overdrive's existing workflow
journal rather than needing a new store. The most valuable *warning* is Fly's own retrospective
admission that "there really wasn't any contract between `flyd` and `init`" — they ran an ad-hoc,
unversioned host↔guest channel in production for years and are now replacing it with `pilot`, an
OCI-compliant runtime with a defined API. Overdrive should write that contract first (I-6).

Two clusters of Fly's design are **actively wrong for Overdrive**. First, the multi-tenancy apparatus
— the jailer, oversubscription, market placement, Anycast/WireGuard edge — exists because Fly rents
compute to adversarial strangers; Overdrive is a single-tenant appliance with an already-built
mTLS dataplane. Second, the all-block-device choice is not a considered tradeoff against virtiofs —
**Firecracker simply has no virtiofs**. Cloud Hypervisor does, which means Overdrive faces a genuine
decision Fly never had to make, and the shortest path to a production-drivable slice
(`overdrive serve` + `overdrive deploy` booting a real VM) is virtiofs + overlayfs, not an LVM
thin-pool manager. Separately: the public `superfly/firecracker` fork is a **2020 v0.24.6 artifact**
and is not what Fly runs — they shipped v1.7.0 fleet-wide in May 2024. Reading patches out of that
repo would be a six-year-stale premise.

## Research Methodology

**Search Strategy**: Targeted queries against Fly's own surfaces — `fly.io/blog`, `fly.io/docs`,
`community.fly.io`, `github.com/superfly` — plus named-author searches (Thomas Ptacek / tqbf, Kurt
Mackey, Will Jordan, JP Phillips). Upstream project docs (firecracker-microvm, containerd) were used
only to establish what a mechanism *is*, never to assert what Fly does. HN was used only where a
comment is staff-attributed and corroborated elsewhere.

**Source Selection**: Fly primary sources (blog / docs / forum / repos) are treated as High
reputation for claims about Fly's own systems. Third-party VMM comparison posts (Northflank,
PandaStack, E2B, kuberns) surfaced repeatedly in search and were **rejected** — they are
commercially interested and speculate about Fly's reasoning without access to it.

**Quality Standards**: Every claim carries an evidence label ([DOCUMENTED] / [INFERRED] /
[COMMUNITY] / [UNVERIFIED]) and a source URL with access date. Every source carries its publication
date, and stale claims are flagged explicitly. Where Fly has published nothing, the section says so
in one line rather than speculating.

## Reading Guide — Evidence Labels

Every claim in this document carries one of:

- **[DOCUMENTED]** — stated explicitly in a Fly.io official source (blog, docs, repo, talk).
- **[INFERRED]** — a reasonable reading of a primary source that does not state it outright.
- **[COMMUNITY]** — HN/forum/third-party claim, including Fly staff speaking informally.
- **[UNVERIFIED]** — could not be confirmed from any source found.

Dates matter: Fly's architecture changed substantially between 2020 and 2026. Each finding carries
the publication date of its source and a staleness flag where relevant.

## 1. The VMM — Firecracker (and the Cloud Hypervisor question)

### 1.1 Firecracker, chosen over Xen; not Cloud Hypervisor — [DOCUMENTED]

**Evidence**: Will Jordan (Fly SRE), "The Serverless Server" (2022-06-30):

> "Unlike Xen, we don't emulate arbitrary devices, but rather virtio devices designed to be
> efficient to implement"

Xen was rejected because it was "designed to run arbitrary operating systems in arbitrary hardware
configurations." Fly cites Firecracker startup under 125 ms and "thousands of micro-VMs on a single
server, paying less than 5MB per instance in memory."

**Source**: [Fly Blog — The Serverless Server](https://fly.io/blog/the-serverless-server/)
(2022-06-30, Will Jordan) — accessed 2026-08-01.
**Confidence**: High.

**On Cloud Hypervisor**: I found **no** Fly primary source that evaluates, adopts, or rejects Cloud
Hypervisor. Fly's public position is Firecracker, consistently, from 2020 through the 2026 docs
("Application code runs in Firecracker microVMs"). **Marking "Fly's view of the
Firecracker-vs-Cloud-Hypervisor tradeoff" as UNVERIFIED — no primary source exists.** Third-party
comparisons exist (Northflank, PandaStack, E2B) but they are not Fly speaking and are commercially
interested; they are not cited as evidence for Fly's reasoning.

**Source (for the current claim)**: [Fly Docs — The Fly.io
Architecture](https://fly.io/docs/reference/architecture/) — accessed 2026-08-01.

### 1.2 Fly does maintain a fork — but the *public* fork is stale — [DOCUMENTED, with a trap]

**Evidence**: `github.com/superfly/firecracker` exists, forked from
`firecracker-microvm/firecracker`, with a release tag **`v0.24.6-fly`**. Firecracker v0.24 is a
2020-era release line.

Meanwhile, Fly announced in their own forum on **2024-05-31** (author `akshit-fly`): they shipped
**Firecracker v1.7.0** fleet-wide, citing "up to 10-15% improved block I/O performance" and
"improved network performance due to the improvements in firecracker device emulation." They also
built "new internal tooling that gives our infrastructure team the ability to feature flag
firecracker release roll-outs."

**Sources**: [GitHub — superfly/firecracker](https://github.com/superfly/firecracker); [Fly Community
— We shipped Firecracker v1.7.0](https://community.fly.io/t/we-shipped-firecracker-v1-7-0/20140)
(2024-05-31) — accessed 2026-08-01.
**Confidence**: High for both facts; **the reconciliation between them is INFERRED**.

**Analysis**: The public `superfly/firecracker` fork is a ~2020 artifact and is **not** what Fly runs
today. Do not read patches out of it and assume they are live. What the pair of facts *does* tell us:

1. Fly ran a pinned, patched, old Firecracker for years (v0.24 line, 2020–~2023).
2. Getting off it was hard enough that reaching v1.7.0 in mid-2024 was newsworthy internally, and
   they built **feature-flagged VMM rollout tooling** as part of the effort.
3. That tooling — per-host, per-machine VMM version rollout with a flag — is itself a design lesson:
   the VMM is a component you will need to roll forward independently of the orchestrator, gradually,
   with a kill switch.

**What patches Fly carries in 2026 is UNVERIFIED.** Fly has not published a patch list. Search did
not surface a Fly statement enumerating their deltas from upstream.

**STALENESS FLAG**: any claim sourced to the public `superfly/firecracker` fork describes 2020, not
2026.

## 2. `flyd` — the per-host orchestration daemon

### 2.1 flyd replaced Nomad; it is a per-worker daemon, not a cluster scheduler — [DOCUMENTED]

**Evidence**: Fly ran HashiCorp Nomad as its orchestrator through roughly 2022. The replacement,
`flyd`, is described in "Carving The Scheduler Out Of Our Orchestrator" (2023-02-01):

> "Flyd is rigidly structured as a collection of state machines, like 'create a machine' or 'delete
> a volume'. Each has a concrete representation both in the code (using Go generics) and in
> `boltdb`."

**Source**: [Fly Blog — Carving The Scheduler Out Of Our
Orchestrator](https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/) (2023-02-01) —
accessed 2026-08-01.
**Confidence**: High.
**Analysis**: The load-bearing architectural inversion is that `flyd` runs **on every worker**, one
instance per physical host, and owns that host's machines. There is no global consensus over machine
placement. This is the "inside-out orchestrator" shape.

### 2.2 State is an append-only log in an embedded BoltDB, per host — [DOCUMENTED]

**Evidence**:

> "Every `flyd` keeps a `boltdb` database of its current state, which is an append-only log of all
> the operations applied to the worker."

and, on crash recovery, `flyd` "picks up right where it left off."

**Source**: [Fly Blog — Carving The
Scheduler](https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/) (2023-02-01) —
accessed 2026-08-01.
**Confidence**: High (single authoritative primary source, stated plainly).
**Analysis**: This is the single most directly transferable idea for Overdrive. `flyd`'s durability
model is: *the state machine's step transitions are the log entries*. Restart replays the log and
resumes the in-flight FSM rather than re-deriving desired state from a cluster store. Compare
Overdrive's reconciler `ViewStore` (redb + CBOR blobs, write-through then in-memory insert): the
same fsync-then-memory ordering concern applies, but Fly's unit is an FSM step, not a reconciler
View.

### 2.3 Scheduling is market/bid-shaped, not bin-packing consensus — [DOCUMENTED]

**Evidence**:

> "Requests to schedule jobs are bids for resources; workers are suppliers. Our orchestrator sits in
> the middle like an exchange."

`flaps` (the Machines API server) "collects capacity information from all the `flyd`s in [a region],
and then runs a quick best-fit ranking over the workers with space."

Nomad was rejected because (a) bin-packing was misaligned with Fly's economics ("We rent out server
space. So we buy enough of them to have headroom in every region."), (b) Nomad's federation model
conflicted with the desire for a single global cluster, and (c) Nomad scheduling is asynchronous
while Fly needed *synchronous* start ("an HTTP request... scale from zero to handle it... starting a
Fly Machine on a particular server, synchronously").

**Source**: [Fly Blog — Carving The
Scheduler](https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/) (2023-02-01) —
accessed 2026-08-01.
**Confidence**: High.
**Analysis**: The synchronous-start requirement is the design driver. A scheduler that returns
"accepted, will converge eventually" cannot serve a request-triggered cold start. Fly's answer was
to make the API call reach a specific worker's `flyd` and block on its FSM.

### 2.4 `flaps` is the Machines API server; the Machine has an explicit lifecycle state set — [DOCUMENTED]

**Evidence**: Fly's docs enumerate Machine states. The documented set includes `created`,
`starting`, `started`, `stopping`, `stopped`, `suspending`, `suspended`, `replacing`, `destroying`,
`destroyed`.

**Source**: [Fly Docs — Machine states and lifecycle](https://fly.io/docs/machines/machine-states/)
— accessed 2026-08-01.
**Confidence**: Medium-High (official docs; exact enumeration should be re-checked against the live
page since Fly adds states — `suspended` was added mid-2024).
**Analysis**: Note the state set is *machine-level*, not process-level: Fly separates "the VM exists
and is stopped" from "the VM is destroyed." Overdrive's allocation states (Pending / Running /
Terminated) collapse these. A microVM driver needs the `stopped`-but-rootfs-preserved state to be
representable, because that is what makes fast restart and scale-to-zero possible.

## 3. Docker image → rootfs — the part Overdrive lacks entirely

This is the best-documented part of Fly's stack and the highest-value section for Overdrive.

### 3.1 OCI pull is done directly, without Docker — [DOCUMENTED]

**Evidence**: Thomas Ptacek, "Docker Without Docker" (2021-04-08):

> "An OCI image is just a stack of tarballs."

The post walks the registry protocol by hand: bearer token from the auth service → manifest list →
per-architecture manifest → layer digests → blob download by SHA256 → "Unpack the tarballs in order
and you've got the filesystem layout."

**Source**: [Fly Blog — Docker Without Docker](https://fly.io/blog/docker-without-docker/)
(2021-04-08, Thomas Ptacek) — accessed 2026-08-01.
**Confidence**: High.

### 3.2 The landing format is block devices, not a shared filesystem — [DOCUMENTED]

**Evidence**:

> "what Firecracker wants is a set of block devices that Linux will mount as it boots up."

**Source**: [Fly Blog — Docker Without Docker](https://fly.io/blog/docker-without-docker/)
(2021-04-08) — accessed 2026-08-01.
**Confidence**: High.
**Analysis**: This is the fork in the road. Firecracker (and Cloud Hypervisor) take `virtio-blk`
devices; a container runtime takes a directory. Turning a flattened layer tree into a *block device*
is the machinery Overdrive does not have. Fly's answer is device-mapper, not virtiofs (see 3.3).

### 3.3 containerd + LVM2 thin pool, giving CoW snapshots per machine — [DOCUMENTED]

**Evidence**:

> "Pull it from the registry into our server-local containerd, configured to run on an LVM2 thin
> pool"

LVM2 thin provisioning gives copy-on-write snapshots so "multiple containers don't step on each
other," and subsequent deploys are "lightning fast" because layers are already cached on the host.

**Source**: [Fly Blog — Docker Without Docker](https://fly.io/blog/docker-without-docker/)
(2021-04-08) — accessed 2026-08-01.
**Confidence**: High for the 2021 architecture. **STALENESS FLAG**: this is a 2021 statement. Fly
has since rebuilt orchestration (flyd, 2022–23) and it is not confirmed that `containerd` is still
in the path in 2026. Treat "containerd is still used" as **UNVERIFIED** for the current era; treat
"LVM2 thin pool + device-mapper CoW" as likely-still-true because the same mechanism underpins
volumes (§7).

**Analysis of the mechanism** — this is the concrete recipe:

1. Pull OCI layers into a content-addressed local store (containerd's).
2. Use the containerd `devmapper` snapshotter, backed by an **LVM2 thin pool** carved from local
   NVMe.
3. Each layer becomes a thin device; each layer's device is a *snapshot* of its parent.
4. The final image layer's snapshot is the base; a per-machine writable snapshot is taken from it.
5. That snapshot device is handed to Firecracker as a `virtio-blk` drive (the rootfs, `/dev/vdb` per
   the init README — see §4).

The filesystem *inside* the thin device is a normal Linux filesystem written by the snapshotter when
it materialises the layers — containerd's devmapper snapshotter formats with **ext4** by default.
**[INFERRED]** for Fly specifically: Fly does not name the filesystem in the post; ext4 is
containerd-devmapper's default and matches the init README's use of ext2 for the *init* device.

**Cross-reference**: containerd's devmapper snapshotter and its thin-pool setup are documented
upstream at
[containerd/docs/snapshotters/devmapper.md](https://github.com/containerd/containerd/blob/main/docs/snapshotters/devmapper.md)
— accessed 2026-08-01.

### 3.4 Cold-path cost — thin evidence

**Evidence**: Fly's suspend/resume docs give a cold start of "~2+ seconds for common apps"; the
"Docker Without Docker" post claims subsequent deploys are "lightning fast" without numbers.

**Source**: [Fly Docs — Suspend/Resume](https://fly.io/docs/reference/suspend-resume/) — accessed
2026-08-01.
**Confidence**: Low. Fly has not published a component breakdown of the cold path (pull vs. snapshot
vs. VMM boot vs. init vs. app). Documented in Knowledge Gaps.

## 4. `init` — the in-guest init process

### 4.1 Fly's init is Rust, open-sourced as `superfly/init-snapshot` — [DOCUMENTED]

**Evidence**: The repo README states the init "powers every Firecracker microvm we run for our
users" and is "tailored for firecracker microvms." It can be packaged as **either a device or an
initrd**.

**Source**: [GitHub — superfly/init-snapshot
README](https://github.com/superfly/init-snapshot/blob/public/README.md) — accessed 2026-08-01.
**Confidence**: High (primary source, Fly's own repo).

### 4.2 The device layout: init on `/dev/vda` (64 MB ext2), rootfs on `/dev/vdb` — [DOCUMENTED]

**Evidence**: The README's device-mode instructions build a **64 MB ext2** filesystem containing
`/fly/init` (the executable) and `/fly/run.json` (the configuration), attach it as `/dev/vda`, and
attach the image rootfs as `/dev/vdb`. A **vsock virtio device** is also attached.

**Source**: [superfly/init-snapshot
README](https://github.com/superfly/init-snapshot/blob/public/README.md) — accessed 2026-08-01.
**Confidence**: High.
**Analysis**: This is the cleanest single design idea in the whole stack for Overdrive's purposes.
**The init is not injected into the customer's image.** It rides on a separate, platform-owned block
device. The guest kernel is booted with `init=/fly/init` (or equivalent), Fly's init runs from
`/dev/vda`, then it mounts `/dev/vdb` and `pivot_root`s / `chroot`s into the customer rootfs before
`exec`ing the entrypoint. Consequences:

- The customer image is never mutated — no layer is appended, no binary is copied in. The image
  snapshot stays a pure CoW child of the cached layers.
- Platform init can be upgraded independently of every customer image.
- Config is a **JSON file on a block device** (`/fly/run.json`), not kernel cmdline, not env, not a
  network fetch. This is a durable, arbitrarily-large, structured config channel that exists before
  networking does.

### 4.3 Configuration is injected as JSON — [DOCUMENTED]

**Evidence**:

> "To configure our init, we're injecting a JSON file into the root device."

The 2021 blog post enumerates what init handles: mounting Linux filesystems, applying injected
configuration (users, network, entrypoints), DNS resolver configuration, spawning an SSH server, and
monitoring the application entrypoint.

**Sources**: [superfly/init-snapshot
README](https://github.com/superfly/init-snapshot/blob/public/README.md); [Fly Blog — Docker Without
Docker](https://fly.io/blog/docker-without-docker/) (2021-04-08) — both accessed 2026-08-01.
**Confidence**: High for the list; Medium for current-era completeness (the 2021 list predates
suspend/resume and checks).

**Note on networking**: "applying injected configuration (users, **network**, entrypoints)" is the
load-bearing phrase for §6. It says the guest NIC is configured **by init from the injected JSON** —
not DHCP, not kernel `ip=`. See §6.3.

### 4.4 The host↔guest channel is vsock; init exposes a Unix socket in-guest — [DOCUMENTED / partly COMMUNITY]

**Evidence**: The README specifies attaching "a vsock virtio device." Search-surfaced description of
the model: `flyd` manages a vsock connection to the init process running in the Machine, and the
init exports a Unix socket available to superuser processes inside the Machine.

**Sources**: [superfly/init-snapshot
README](https://github.com/superfly/init-snapshot/blob/public/README.md); Fly community/docs
references surfaced via search — accessed 2026-08-01.
**Confidence**: Medium. The vsock device is documented; the *protocol* over it is not published.

**What is NOT documented** — flagged **UNVERIFIED**:

- The wire protocol/RPC framing over vsock.
- How exit codes are reported (though vsock is the obvious channel).
- Whether logs are shipped over vsock or over the network (Fly runs a log shipper; whether the guest
  side is init or a separate agent is not stated in the open README).
- The precise "workload is up" signal. `flyd` clearly learns machine state, and Fly Proxy separately
  does health checks over the network. Whether readiness comes from init-over-vsock or from proxy
  health checks is not confirmed in a primary source.

## 5. Kernel — thin evidence

Fly has published very little about its guest kernel. What exists:

### 5.1 The guest kernel is platform-supplied, versioned, and not user-selectable — [DOCUMENTED]

**Evidence**: A Fly "Fresh Produce" post announces a bump of the **default kernel version to 5.15.98
from 5.15.93** for new Machines (June 2023). A separate wishlist thread ("Custom kernel with Fly
Machines?") exists precisely because users *cannot* supply their own kernel.

**Sources**: [Fly Community — Updated default kernel
version](https://community.fly.io/t/updated-default-kernel-version/13786) (2023-06); [Fly Community
— Custom kernel with Fly Machines?](https://community.fly.io/t/custom-kernel-with-fly-machines/6082)
— accessed 2026-08-01.
**Confidence**: Medium-High (Fly's own forum, staff-posted release note).
**STALENESS FLAG**: 5.15.x is a 2023 data point. The 2026 default is **UNVERIFIED**.

### 5.2 Direct kernel boot, not a bootloader — [INFERRED from Firecracker's model]

Firecracker only supports direct kernel boot: an uncompressed ELF `vmlinux` (x86_64 also accepts
bzImage; vmlinux recommended), a `boot_args` kernel command line set via `PUT /boot-source`, and
optionally an initrd. There is no BIOS/UEFI/GRUB path. Fly therefore necessarily does direct kernel
boot.

**Source**: [firecracker-microvm/firecracker — rootfs and kernel
setup](https://github.com/firecracker-microvm/firecracker/blob/main/docs/rootfs-and-kernel-setup.md)
— accessed 2026-08-01.
**Confidence**: High for the Firecracker constraint; the *specifics of Fly's* kernel `.config`,
cmdline string, and whether they use an initrd are **UNVERIFIED**.

### 5.3 initrd vs. none — [PARTIALLY DOCUMENTED]

The `superfly/init-snapshot` README says the init "can be packaged as either a device or an initrd,"
and documents the **device** path in detail (`/dev/vda` 64 MB ext2). That is weak evidence that
production uses the device path; the README does not say which Fly runs.

**Source**: [superfly/init-snapshot
README](https://github.com/superfly/init-snapshot/blob/public/README.md) — accessed 2026-08-01.
**Confidence**: Low on which one production uses.

### 5.4 Boot-time numbers and what dominates them — [DOCUMENTED, but coarse]

The only published numbers: Firecracker startup "under 125 ms" (Fly, 2022, citing the VMM's own
figure — this is VMM-to-guest-kernel-start, not app-ready); cold start "~2+ seconds for common apps"
and resume-from-suspend "a few hundred ms" (Fly docs, 2024+).

**Sources**: [Fly Blog — The Serverless Server](https://fly.io/blog/the-serverless-server/)
(2022-06-30); [Fly Docs — Suspend/Resume](https://fly.io/docs/reference/suspend-resume/) — accessed
2026-08-01.
**Analysis**: The gap between 125 ms and 2+ s is the interesting part and Fly has **not** broken it
down. The 125 ms is VMM+kernel; the remaining ~1.9 s is image/device setup, init, and the
application's own startup. **A component-level cold-path breakdown is a documented Knowledge Gap.**

## 6. Networking

### 6.1 The host side: WireGuard mesh + eBPF routing + Anycast/`fly-proxy` — [DOCUMENTED]

**Evidence**: Thomas Ptacek, "Incoming! 6PN Private Networks" (2020-12-08):

> "Fly.io is fully connected through a WireGuard mesh joining every point in our network where
> services can run."

> "We route with a sequence of small BPF programs; they enforce access control... and do some silly
> address rewriting footwork so that we can use WireGuard's cryptokey routing to get packets from
> one host to another."

The edge: Fly Docs describe **BGP Anycast** ("We broadcast and accept traffic from ranges of IP
addresses (both IPv4 and IPv6) in all our datacenters"), a per-server Rust proxy ("Every server in
our infrastructure runs a Rust-based proxy named `fly-proxy`") handling client connections and TLS
termination, and WireGuard "backhaul" tunnels between datacenters.

**Sources**: [Fly Blog — Incoming! 6PN Private
Networks](https://fly.io/blog/incoming-6pn-private-networks/) (2020-12-08, Thomas Ptacek); [Fly Docs
— Architecture](https://fly.io/docs/reference/architecture/) — accessed 2026-08-01.
**Confidence**: High.

### 6.2 6PN: routing information is encoded *into the IPv6 address* — [DOCUMENTED]

**Evidence**: 6PN addresses (prefix `fdaa::/16`) embed "an identifier for your organization, an
identifier for the Fly host that your app is running on, and an identifier for the individual
instance of your app." Each Machine gets a 6PN address exposed in-guest as `fly-local-6pn` in
`/etc/hosts`.

Service discovery: "our service discovery system populates a database on each host that we run a
Rust DNS server off of, to serve the 'internal' domain. We inject the IP of that DNS server into
your `resolv.conf` — the IP address of that server is always `fdaa::3`."

**Source**: [Fly Blog — Incoming! 6PN Private
Networks](https://fly.io/blog/incoming-6pn-private-networks/) (2020-12-08) — accessed 2026-08-01.
**Confidence**: High.

**Analysis**: This is a strikingly Overdrive-relevant pattern and also a **cautionary tale**. The
upside: address-embedded routing means the eBPF forwarder needs no lookup table for the host hop — it
reads the destination host out of the destination address. The downside, which Fly hit head-on: it
makes an address **host-bound**, so migrating a Machine between hosts changes its address. From
"Making Machines Move" (2024-07-30):

> Migration required either implementing network address mappings or "burning several weeks doing
> the direct configuration fix fleet-wide" for Postgres cluster configurations using literal IPv6
> addresses.

**Source**: [Fly Blog — Making Machines Move](https://fly.io/blog/machine-migrations/) (2024-07-30,
Thomas Ptacek) — accessed 2026-08-01.

Overdrive's per-workload `/30` `workload_addr` has the same property — it is host-local by
construction. Fly's experience says: **do not let the workload's stable identity be its address.**
Fly's own east-west answer is DNS-first (`resolv.conf` injected, host-local DNS server at `fdaa::3`),
which matches Overdrive's ADR-0072 dial-by-name direction.

### 6.3 The guest side: a virtio NIC configured by `init` from injected JSON — [INFERRED, with documented support]

**Evidence**: The 2021 "Docker Without Docker" post lists init's duties as "applying injected
configuration (users, **network**, entrypoints)" and "DNS resolver configuration." The init README
confirms configuration arrives as `/fly/run.json` on the init block device.

**Sources**: [Fly Blog — Docker Without Docker](https://fly.io/blog/docker-without-docker/)
(2021-04-08); [superfly/init-snapshot
README](https://github.com/superfly/init-snapshot/blob/public/README.md) — accessed 2026-08-01.
**Confidence**: Medium-High. The *mechanism* (init applies network config from JSON) is documented;
the *details* (interface naming, exact address/route/MTU fields, IPv4 handling) are not published.

**Explicitly NOT the mechanism** — no Fly source mentions **DHCP** or kernel-cmdline `ip=` for guest
network configuration, and both would be odd choices given a JSON config channel already exists on
`/dev/vda`. **DHCP/`ip=` absence is INFERRED, not proven.**

**Host side of the link**: Firecracker's only network device is `virtio-net` backed by a host **tap**
device; the host end of the tap is what Fly's eBPF programs attach to. This is Firecracker's
documented model, not a Fly-specific statement. **[INFERRED for Fly; DOCUMENTED for Firecracker.]**
Fly has not published its tap-per-machine naming/lifecycle scheme.

**Source (Firecracker network model)**:
[firecracker-microvm/firecracker — network setup
docs](https://github.com/firecracker-microvm/firecracker/blob/main/docs/network-setup.md) — accessed
2026-08-01.

## 7. Storage / volumes

### 7.1 Volumes are host-local NVMe slices via LVM thin pools — [DOCUMENTED]

**Evidence**: Fly's docs state a Fly Volume "is a slice of an NVMe drive on the same physical server
as the Machine on which it's mounted"; each volume "exists on one server in a single region and is
not network storage." On volume-eligible servers Fly uses "Linux LVM to carve out a thin pool," and
thin volumes "allocate as data is written and have in effect a current size and a cap."

A Fly staff comment on Hacker News confirms: "Fly Volumes are attached NVME storage; they're anchored
to the physical host."

**Sources**: [Fly Docs — Fly Volumes overview](https://fly.io/docs/volumes/overview/); [Fly Blog —
Persistent Storage and Fast Remote
Builds](https://fly.io/blog/persistent-storage-and-fast-remote-builds/); [Hacker News comment
38661624](https://news.ycombinator.com/item?id=38661624) — accessed 2026-08-01.
**Confidence**: High (official docs + blog + staff comment, three independent surfaces).
**Analysis**: A volume is a `virtio-blk` device, same as the rootfs — **not virtiofs**. One volume
per Machine, one Machine per volume. This keeps the guest-side story uniform (everything is a block
device) and avoids a host-side FUSE/virtiofsd daemon per VM.

### 7.2 Volumes are block devices, encrypted with LUKS2 — [DOCUMENTED]

**Evidence**: "Making Machines Move" describes per-volume LUKS2 encryption and a migration hazard:

> "Different workers running different cryptsetup versions default to different LUKS2 header sizes
> (4MiB vs 16MiB), requiring 'an RPC call that carries metadata about the designed LUKS2
> configuration for the target VM.'"

**Source**: [Fly Blog — Making Machines Move](https://fly.io/blog/machine-migrations/) (2024-07-30) —
accessed 2026-08-01.
**Confidence**: High.

### 7.3 Migration uses `dm-clone` over iSCSI, after abandoning NBD — [DOCUMENTED]

**Evidence**: Fly moves Machines between hosts by creating a `dm-clone` target on the destination:

> "when the new Fly Machine tries to read from it, the block storage system works out whether the
> block has been transferred. If it hasn't, it's fetched over the network from the original volume;
> this is called 'hydration'."

Transport: they started with NBD and switched to iSCSI because "we kept getting stuck nbd kernel
threads when there was any kind of network disruption." `fstrim` + `DISCARD` lets `dm-clone` mark
unused blocks hydrated without copying them.

The migration sequence: source stops the Machine → target `flyd` receives the clone request → target
starts `dm-clone` replicating over iSCSI → new Machine boots on the target against the clone device →
hydration completes and the device converts to a plain linear device.

**Source**: [Fly Blog — Making Machines Move](https://fly.io/blog/machine-migrations/) (2024-07-30,
Thomas Ptacek) — accessed 2026-08-01.
**Confidence**: High.

### 7.4 Why block devices and not virtiofs — [INFERRED]

Fly has **not** published a "we chose virtio-blk over virtiofs because…" statement. **UNVERIFIED as
a stated rationale.** What the evidence supports as *observed* design:

- Everything the guest touches — init device, rootfs, volume — is a `virtio-blk` device.
- Firecracker **does not implement virtiofs at all** (its device set is virtio-net, virtio-blk,
  virtio-vsock, virtio-balloon, serial, i8042). So for Fly the question never arose: choosing
  Firecracker *is* choosing block devices. Cloud Hypervisor, by contrast, **does** support virtiofs.
- The whole CoW/snapshot/migration story (LVM thin pool, `dm-clone`, LUKS2, `fstrim`) lives at the
  block layer. A filesystem-passthrough model would have none of it.

**Source**: [Firecracker
design/features](https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md) —
accessed 2026-08-01.
**Analysis**: For Overdrive this is a genuine fork in the road that Fly's evidence *does not* decide,
because Overdrive is targeting Cloud Hypervisor, which has virtiofs. See Implications §I-4.

## 8. Suspend / resume & snapshots

### 8.1 Suspend is a Firecracker snapshot to Fly-managed storage — [DOCUMENTED]

**Evidence**: Fly docs: suspend captures "CPU registers, memory contents, open file handles."
Snapshots are stored by Fly, not on the user's volume. Resume: "a few hundred ms" vs. cold start
"~2+ seconds for common apps." Machines must be ≤ 2 GB memory to be suspend-eligible. Suspend "does
not reset the machine's `rootfs`" (unlike stop→start). Machines must have been updated after
2024-06-20 20:00 UTC.

**Source**: [Fly Docs — Machine Suspend and Resume](https://fly.io/docs/reference/suspend-resume/) —
accessed 2026-08-01.
**Confidence**: High.

### 8.2 Resume path: new Firecracker process + snapshot load via the API socket — [DOCUMENTED/COMMUNITY]

**Evidence**: Community/Fresh-Produce description: on resume, Fly "starts a new Firecracker process
and then makes an HTTP request to its API unix socket to load the previously saved snapshot."

**Source**: [Fly Community — More reliable Machine
resumes](https://community.fly.io/t/more-reliable-machine-resumes/26007) — accessed 2026-08-01.
**Confidence**: Medium-High (Fly staff post in Fly's own forum, but not the docs).
**Analysis**: This matches Firecracker's documented `LoadSnapshot` API. Firecracker's snapshot-load
supports both a full memory read and a **userfaultfd-backed** demand-paged restore; whether Fly uses
the UFFD path is **UNVERIFIED** — Fly has not published this. The "few hundred ms" resume for up to
2 GB is more consistent with demand paging or a local-file mmap than with a synchronous 2 GB read
from remote storage, but that is inference, not evidence.

### 8.3 Snapshots are invalidated by deploys; suspend does not free capacity — [DOCUMENTED]

**Evidence**: "Snapshots are tied to the exact code and state of the machine they were taken from.
If you deploy new code, the old snapshot can't be resumed safely and will be discarded." And:
suspension "does not free capacity in a region."

**Source**: [Fly Docs — Suspend/Resume](https://fly.io/docs/reference/suspend-resume/) — accessed
2026-08-01.
**Confidence**: High.
**Analysis**: The capacity note is important and counter-intuitive: suspend is a *latency*
optimisation, not a *density* one. Fly still reserves the machine's memory footprint. Scale-to-zero
via `stop` frees capacity; suspend does not.

### 8.4 Autosuspend is proxy-driven — [DOCUMENTED]

**Evidence**: `auto_stop_machines = "suspend"` in `fly.toml`; Fly Proxy "checks for idle periods
every few minutes and automatically suspends during low traffic, resuming when requests arrive."

**Source**: [Fly Docs — Suspend/Resume](https://fly.io/docs/reference/suspend-resume/); [Fly
Community — Autosuspend is
here!](https://community.fly.io/t/autosuspend-is-here-machine-suspension-is-enabled-everywhere/20942)
— accessed 2026-08-01.
**Confidence**: High.
**Analysis**: The *proxy* owns the scale-to-zero decision, not the orchestrator. This is the
"inside-out" pattern again — the component with the traffic signal makes the lifecycle call, and
`flyd` just executes it. Overdrive's equivalent signal source would be the dataplane / fly-proxy
analogue, not a reconciler tick.

## 9. Multi-tenancy & isolation

### 9.1 The boundary is the hypervisor; tenants never share a kernel — [DOCUMENTED]

**Evidence**: Fly's security docs: apps "run inside Firecracker, a memory-safe KVM hypervisor,
turning container images from users into VMs for full, no-shared-kernel isolation between
applications. Tenants never share kernels."

**Source**: [Fly Docs — Security](https://fly.io/docs/security/); [Fly —
Security](https://fly.io/security/) — accessed 2026-08-01.
**Confidence**: High.

### 9.2 Defence in depth: seccomp-bpf (~40 syscalls) + external jailer — [DOCUMENTED]

**Evidence**: Thomas Ptacek, "Sandboxing and Workload Isolation" (2020-07-29):

> the Firecracker VMM "seccomp-bpf's itself down to something like 40 system calls," several with
> "tight argument filters"

> it "runs itself under an external jailer that chroots, namespaces, and drops privileges"

Device code size is cited as the point: block device ~1,400 lines of Rust including tests, network
driver ~700 lines pre-tests. "The Firecracker VMM is tiny, easily readable, and deliberately
implements the minimal number of concepts."

Resource limits: Firecracker "enforces configured CPU and memory resource limits with cgroups" (Fly,
2022).

**Sources**: [Fly Blog — Sandboxing and Workload
Isolation](https://fly.io/blog/sandboxing-and-workload-isolation/) (2020-07-29, Thomas Ptacek); [Fly
Blog — The Serverless Server](https://fly.io/blog/the-serverless-server/) (2022-06-30) — accessed
2026-08-01.
**Confidence**: High.

### 9.3 Fly's stated threat model puts the *network* above the hypervisor — [DOCUMENTED]

**Evidence**:

> "the most important attack surface you need to reduce is exposure to your network"

**Source**: [Fly Blog — Sandboxing and Workload
Isolation](https://fly.io/blog/sandboxing-and-workload-isolation/) (2020-07-29) — accessed
2026-08-01.
**Analysis**: This is a *stated priority ordering*, and it is worth taking seriously: Fly's own
security people argue the realistic breach path in a multi-tenant compute platform is lateral
movement across the internal network, not a Firecracker escape. That ordering is exactly why 6PN's
eBPF programs "enforce access control" as their first named job (§6.1).

### 9.4 Incidents — no public hypervisor-escape incident found

**Evidence**: Searches for Fly security incidents/CVEs surfaced no published hypervisor-escape or
cross-tenant compromise. Fly maintains public security and compliance pages and a shared-
responsibility model doc.

**Sources**: [Fly Docs — Security practices and
compliance](https://fly.io/docs/security/security-at-fly-io/); [Fly Docs — Shared responsibility
model](https://fly.io/docs/security/shared-responsibility/) — accessed 2026-08-01.
**Confidence**: Low as a *negative* claim. Absence of search results is not evidence of absence.
**Marked UNVERIFIED**: "Fly has had no isolation-boundary security incident." Documented in
Knowledge Gaps.

## 10. What went wrong — postmortems and rebuilds

**This is the highest-signal section.** Fly is unusually candid, and nearly every rebuild has the
same shape: *a HashiCorp component designed for one datacenter and one team was stretched to a
global multi-tenant platform, and broke on the fan-out.*

### 10.1 Consul: 10 Gb/s of service-discovery traffic for a tiny dataset — [DOCUMENTED]

**Evidence**: Thomas Ptacek, "A Foolish Consistency" (2022-03-29):

> "we found ourselves driving over 10 (t-e-n) gb/sec of Consul traffic across our fleet."

The mechanism: Fly had to long-poll individual server endpoints because Consul's catalog API didn't
give per-instance metadata efficiently — an N² polling problem at tens of thousands of services.
"incremental changes, which happen every few seconds, are expensive," because any change triggered a
full refresh across every long-polling connection. Consul was "designed to make it easy to manage a
single engineering team's applications," not thousands of tenants globally.

Their interim hacks are instructive: `consul-templaterb` wrote "giant JSON blobs to disk," and
"fly-proxy gets a signal and re-reads it into memory." They then built `attache`, holding "a local
sqlite cache of all of Consul's state" so infra services queried the local cache instead of Consul.

**Source**: [Fly Blog — A Foolish Consistency](https://fly.io/blog/a-foolish-consistency/)
(2022-03-29, Thomas Ptacek) — accessed 2026-08-01.
**Confidence**: High.

**Lesson**: the failure was **an API-shape mismatch amplified by fan-out**, not a scale limit in
Consul's storage. A watch API that returns full state on any change is O(changes × watchers ×
state-size). Overdrive's observation layer (Corrosion/CR-SQLite) must be judged on the same
axis — and note that Overdrive's existing `subscribe_all()` lossy-subscription hygiene issue is
literally the same class of problem seen from the other side.

### 10.2 Corrosion: the replacement was itself immature and corrupted state — [DOCUMENTED]

**Evidence**: Kurt Mackey, "Reliability, it's not great" (2023-03-06):

> "The problem with Corrosion is that it's new and gossip based consistency is a difficult problem."

Fly shipped Corrosion (gossip/CRDT service discovery, the direct ancestor of the CR-SQLite-based
system Overdrive also uses) and it produced corrupted global service-discovery state in production.

**Source**: [Fly Community — Reliability, it's not
great](https://community.fly.io/t/reliability-its-not-great/11253) (2023-03-06, Kurt Mackey) —
accessed 2026-08-01.
**Confidence**: High.
**Lesson, directly applicable to Overdrive**: replacing a consistent store with an eventually-
consistent gossip layer trades one failure mode (stale/centralised) for another (divergent/corrupt).
Overdrive already carries this bet. Fly's experience says the mitigation is not "gossip harder" but
*bounding the blast radius of a wrong row* — which is what Overdrive's "observation rows converge;
intent refuses to start" asymmetry (`.claude/rules/development.md` § rkyv schema evolution) is for.

### 10.3 Nomad: churn amplification and the wrong scheduling model — [DOCUMENTED]

**Evidence**: Kurt Mackey (2023-03-06):

> "Because Nomad creates entirely new instances for each deploy, there's a lot of service discovery
> churn; many, many event updates per second."

Plus the three structural mismatches from the flyd post (§2.3): bin-packing vs. headroom economics,
federation vs. single global cluster, async scheduling vs. synchronous cold start.

**Sources**: [Fly Community — Reliability, it's not
great](https://community.fly.io/t/reliability-its-not-great/11253) (2023-03-06); [Fly Blog — Carving
The Scheduler](https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/) (2023-02-01) —
accessed 2026-08-01.
**Confidence**: High.

**Lesson**: **immutable-replace-on-deploy is a churn amplifier.** Fly's fix was Machines with
**in-place updates** — the Machine is a durable object whose rootfs is swapped, rather than a new
allocation with a new identity. Every replace generates a discovery event fan-out; in-place update
generates none.

### 10.4 Vault: a single US region for secrets, on a global platform — [DOCUMENTED]

**Evidence**: "Vault is in the US, internet connectivity between distant regions (like MAA) and the
US can cause secret lookups to fail."

**Source**: [Fly Community — Reliability, it's not
great](https://community.fly.io/t/reliability-its-not-great/11253) (2023-03-06) — accessed
2026-08-01.
**Confidence**: High.
**Lesson**: anything on the **machine-start critical path** must be host-local or region-local. A
remote secret fetch turns a start into a distributed transaction.

### 10.5 Stolon/Consul-coupled Postgres — [DOCUMENTED]

**Evidence**: "our Postgres clusters have had two major problems: (1) our reliance on Stolon and
live connections to Consul clusters"; they migrated to `repmgr`.

**Source**: [Fly Community — Reliability, it's not
great](https://community.fly.io/t/reliability-its-not-great/11253) (2023-03-06) — accessed
2026-08-01.
**Confidence**: High.
**Lesson**: a data-plane component that requires a **live** connection to the control plane to stay
healthy inherits every control-plane outage.

### 10.6 Root cause Fly names for the whole era: growth outran platform maturity — [DOCUMENTED]

**Evidence**: Kurt attributes the 2022–23 reliability collapse to explosive post-Heroku-exodus
growth — 30% monthly vs. 15% before — outpacing platform maturity and engineering capacity.
Contemporary reporting confirms the CEO's "not great" framing.

**Sources**: [Fly Community — Reliability, it's not
great](https://community.fly.io/t/reliability-its-not-great/11253) (2023-03-06); [DevClass — Fly.io
CEO says reliability 'not
great'](https://devclass.com/2023/03/07/fly-io-ceo-says-reliability-not-great-as-platform-suffers-scaling-issues/)
(2023-03-07) — accessed 2026-08-01.
**Confidence**: High (primary + independent trade press).

### 10.7 The `flyd`↔`init` contract gap, and `pilot` — [DOCUMENTED, and the most Overdrive-relevant admission]

**Evidence**: JP Phillips exit interview (2025-02-12):

> "there really wasn't any contract between `flyd` and `init`"

and the fix:

> "Having `pilot` be an OCI-compliant runtime with an API for `flyd` to drive is a big win for the
> future."

He also defends BoltDB over SQLite for `flyd` state — "I've never lost a second of sleep worried
that someone is about to run a SQL update statement" — while suggesting **per-Machine SQLite
databases** as a future improvement.

**Source**: [Fly Blog — The Exit Interview: JP Phillips](https://fly.io/blog/the-exit-interview-jp/)
(2025-02-12) — accessed 2026-08-01.
**Confidence**: High.

**Analysis — read this one twice.** Fly built a bespoke guest init with an ad-hoc, undocumented
host↔guest channel, ran it in production for years, and their own engineer's retrospective verdict
is that the *missing contract* between the supervisor and the in-guest agent was the limiting factor.
The fix is to make the in-guest component a **runtime with a defined API** (`pilot`, OCI-compliant)
that the supervisor drives. That is: the host↔guest boundary should be a **port trait with a written
behavioural contract**, not a pile of JSON and an unversioned vsock protocol.

`pilot` is current-era and thinly documented publicly. **Details of the `pilot` API surface are
UNVERIFIED**; only its existence and framing come from a primary source.

## Source Analysis

| Source | Domain | Reputation | Type | Date | Access | Cross-verified |
|---|---|---|---|---|---|---|
| Carving The Scheduler Out Of Our Orchestrator | fly.io/blog | High | Official/primary | 2023-02-01 | 2026-08-01 | Y (§10.3, machine-migrations) |
| Docker Without Docker (Ptacek) | fly.io/blog | High | Official/primary | 2021-04-08 | 2026-08-01 | Y (init README) |
| Sandboxing and Workload Isolation (Ptacek) | fly.io/blog | High | Official/primary | 2020-07-29 | 2026-08-01 | Y (Fly security docs) |
| The Serverless Server (Will Jordan) | fly.io/blog | High | Official/primary | 2022-06-30 | 2026-08-01 | Y (Fly docs architecture) |
| Incoming! 6PN Private Networks (Ptacek) | fly.io/blog | High | Official/primary | 2020-12-08 | 2026-08-01 | Y (Fly docs private-networking) |
| A Foolish Consistency (Ptacek) | fly.io/blog | High | Official/primary | 2022-03-29 | 2026-08-01 | Y (Kurt's reliability post) |
| Making Machines Move (Ptacek) | fly.io/blog | High | Official/primary | 2024-07-30 | 2026-08-01 | Y (flyd/BoltDB claim) |
| The Exit Interview: JP Phillips | fly.io/blog | High | Official/primary | 2025-02-12 | 2026-08-01 | Y (flyd BoltDB, Nomad) |
| superfly/init-snapshot README | github.com | High | Official/primary (repo) | n/d (public branch) | 2026-08-01 | Y (Docker Without Docker) |
| superfly/firecracker | github.com | High | Official/primary (repo) | ~2020 tag | 2026-08-01 | N (contradicts v1.7.0 post) |
| Fly Docs — Suspend/Resume | fly.io/docs | High | Official docs | current | 2026-08-01 | Y (community autosuspend) |
| Fly Docs — Machine states and lifecycle | fly.io/docs | High | Official docs | current | 2026-08-01 | N |
| Fly Docs — Architecture | fly.io/docs | High | Official docs | current | 2026-08-01 | Y |
| Fly Docs — Volumes overview | fly.io/docs | High | Official docs | current | 2026-08-01 | Y (HN staff comment, blog) |
| Fly Docs — Security / Shared responsibility | fly.io/docs | High | Official docs | current | 2026-08-01 | Y (2020 sandboxing post) |
| Reliability, it's not great (Kurt Mackey) | community.fly.io | High | Official/primary (CEO) | 2023-03-06 | 2026-08-01 | Y (DevClass) |
| We shipped Firecracker v1.7.0 | community.fly.io | Medium-High | Official forum (staff) | 2024-05-31 | 2026-08-01 | N |
| Updated default kernel version | community.fly.io | Medium-High | Official forum (staff) | 2023-06 | 2026-08-01 | N |
| More reliable Machine resumes | community.fly.io | Medium-High | Official forum (staff) | n/d | 2026-08-01 | Y (Firecracker LoadSnapshot API) |
| Autosuspend is here! | community.fly.io | Medium-High | Official forum (staff) | 2024 | 2026-08-01 | Y (Fly docs) |
| firecracker-microvm/firecracker docs | github.com | High | Official (upstream project) | current | 2026-08-01 | Y |
| containerd devmapper snapshotter docs | github.com | High | Official (CNCF project) | current | 2026-08-01 | Partial |
| HN comment 38661624 (Fly staff, volumes) | news.ycombinator.com | Medium | Community (staff-attributed) | 2023-12 | 2026-08-01 | Y (Fly docs) |
| DevClass — Fly.io CEO reliability | devclass.com | Medium-High | Trade press | 2023-03-07 | 2026-08-01 | Y (Kurt's post) |

**Reputation distribution**: High: 18 (72%) · Medium-High: 5 (20%) · Medium: 2 (8%) · Average ≈ 0.93.
**All sources are Fly primary sources, upstream project docs, or one independent trade-press
confirmation.** No unverified blogs, no SEO content farms, no vendor comparison posts were cited as
evidence (several surfaced in search — Northflank, PandaStack, E2B, kuberns — and were **rejected**
as commercially interested third parties speaking about Fly's reasoning without access to it).

## Knowledge Gaps

### Gap 1: Fly's current Firecracker patch set
**Issue**: Fly runs a patched Firecracker (v1.7.0+ since 2024-05) but has never published its
deltas from upstream. The public `superfly/firecracker` fork is a ~2020 v0.24.6 artifact and is
misleading.
**Attempted**: GitHub repo inspection, HN search, Fly blog/community search.
**Recommendation**: If this matters, inspect the fork's commit graph against upstream tags directly
(`git log upstream/v0.24.6..superfly/master`) and treat the result as *historical*, not current.

### Gap 2: The vsock protocol between `flyd` and `init` (and now `pilot`)
**Issue**: The channel is documented to exist; the wire protocol, framing, versioning, exit-code
reporting, and readiness signalling are not published. `pilot`'s "OCI-compliant runtime with an API"
is named but not specified publicly.
**Attempted**: init-snapshot README, Docker Without Docker, exit interview, community search.
**Recommendation**: Read `superfly/init-snapshot` source (`lib.rs`, which the README itself points at
for the `run.json` schema) rather than looking for prose. This is the single highest-value follow-up
for Overdrive.

### Gap 3: Cold-path latency breakdown
**Issue**: Only endpoints are published (125 ms VMM/kernel; ~2+ s app-ready cold start; few-hundred-ms
resume). No component breakdown — image pull vs. thin-snapshot creation vs. VMM boot vs. init vs. app.
**Attempted**: Fly blog, docs, community search on boot times.
**Recommendation**: Measure it yourself on Cloud Hypervisor; do not budget against Fly's numbers.

### Gap 4: Whether Fly uses userfaultfd for snapshot restore
**Issue**: Firecracker supports UFFD-backed demand-paged restore. Fly's "few hundred ms" resume for
up to 2 GB is *consistent with* UFFD but Fly has not said so.
**Attempted**: Suspend/resume docs, community resume threads.
**Recommendation**: Treat as open. The 2 GB eligibility cap is itself weak evidence *against* a fully
demand-paged restore (a UFFD restore would be far less memory-size-sensitive).

### Gap 5: Guest NIC configuration mechanism, precisely
**Issue**: "init applies injected network config" is documented; the field schema, interface naming,
IPv4 story, and whether anything ever uses DHCP are not.
**Attempted**: 6PN post, Docker Without Docker, init README, private-networking docs.
**Recommendation**: Same as Gap 2 — read `run.json`'s schema in the repo source.

### Gap 6: Whether containerd is still in the 2026 image path
**Issue**: The containerd + LVM2-thin-pool claim is from 2021 and predates flyd. LVM thin pools are
clearly still in use (volumes, `dm-clone`); *containerd specifically* is not confirmed post-rebuild.
**Attempted**: Architecture docs, flyd posts, exit interview.
**Recommendation**: Treat "block devices from an LVM2 thin pool" as durable; treat "containerd
devmapper snapshotter" as a plausible-but-dated implementation detail.

### Gap 7: Security incidents at the isolation boundary
**Issue**: No public Fly hypervisor-escape or cross-tenant incident found. This is a negative result
from search, not a verified absence.
**Attempted**: Security docs, blog, general search.
**Recommendation**: Do not cite "Fly has never had an isolation incident" as a fact.

## Conflicting Information

### Conflict 1: Which Firecracker version does Fly run?
**Position A**: v0.24.6 — the tag on the public `superfly/firecracker` fork.
Source: [github.com/superfly/firecracker](https://github.com/superfly/firecracker). Reputation: High
(Fly's own repo). Evidence: release tag `v0.24.6-fly`.
**Position B**: v1.7.0 as of 2024-05-31, with feature-flagged rollout tooling for future bumps.
Source: [Fly Community — We shipped Firecracker
v1.7.0](https://community.fly.io/t/we-shipped-firecracker-v1-7-0/20140). Reputation: Medium-High (Fly
staff, Fly's forum). Evidence: "up to 10-15% improved block I/O performance."
**Assessment**: **Position B wins.** It is dated, specific, describes a fleet-wide rollout, and
post-dates the fork tag by ~4 years. The public fork is an abandoned mirror. Anyone reading Fly's
Firecracker patches out of the public repo is reading 2020.

### Conflict 2: Is suspend a scale-to-zero mechanism?
**Position A (marketing framing)**: Fly's product pages describe Machines that "scale to zero when
idle," with autosuspend presented alongside it.
Source: [fly.io/learn/firecracker-vm](https://fly.io/learn/firecracker-vm/).
**Position B (docs)**: suspension "does not free capacity in a region."
Source: [Fly Docs — Suspend/Resume](https://fly.io/docs/reference/suspend-resume/).
**Assessment**: **Position B is the operative engineering fact.** Suspend trades a resume-latency win
for zero density win; `stop` is the density mechanism. Both are true of different verbs; the
marketing framing blurs them.

## Implications for an Overdrive Cloud Hypervisor Driver

Overdrive's starting position: a `Driver` port trait, a working exec driver, per-workload
netns + veth + nft-TPROXY transparent mTLS, cgroup v2 management, **no image or rootfs machinery at
all**, single-tenant appliance (not multi-tenant public cloud), Cloud Hypervisor (not Firecracker).

### I-1. COPY: the platform-owned init on its own block device (`/dev/vda`), never injected into the image

This is the single best idea in Fly's stack and it maps onto Overdrive with zero friction.

- Build one small ext4 (Fly uses ext2; ext4 is fine) image containing `overdrive-init` +
  `config.json`. It is **built once, at appliance build time**, not per workload.
- Attach it as the first `virtio-blk` device; attach the workload rootfs as the second.
- Boot with `init=/fly/init`-equivalent on the kernel cmdline. Init mounts the rootfs, applies
  config, `pivot_root`s, `exec`s the entrypoint.
- **Never mutate the customer/workload image.** The rootfs snapshot stays a pure CoW child of the
  cached layers, which is what makes both fast start *and* deterministic content addressing possible.
- Config is a JSON blob on a block device — available before networking, arbitrarily large,
  structured, and versionable. Vastly better than kernel cmdline (length-limited, unstructured) or a
  network fetch (adds a dependency to the boot critical path — see Fly's Vault failure, §10.4).

**Cloud Hypervisor note**: CH takes `--kernel`, `--cmdline`, `--disk path=...` (repeatable), and
supports `virtio-vsock` — every primitive this design needs is present.

### I-2. COPY: the FSM-per-operation + append-only local log, but map it onto Overdrive's existing shape

`flyd`'s durability model — "a collection of state machines... with the transition steps recorded
carefully in a BoltDB database," append-only, resume-where-you-left-off — is the right model for
microVM lifecycle operations, and it is **not** what a reconciler gives you.

Per `.claude/rules/workflows.md`, a microVM `create` is: pull image → materialise rootfs device →
create netns/veth → mint SVID → boot VMM → wait for init ready. That is a **terminating, ordered
sequence of ≥2 side-effecting steps where a crash must not repeat completed steps** — the workflow
candidacy test, met exactly. Do **not** smear it across reconciler ticks with a phase enum in the
`View`; that is the named anti-pattern in that rule and it is precisely what Fly avoided.

So: **`Action::StartWorkflow` driving a `MicroVmProvision` workflow on the existing journal engine**,
not a new bespoke store. Overdrive already has ADR-0066's `workflow-journal.redb`. Fly's BoltDB is
Overdrive's journal; do not build a second one. (JP's own retrospective wish was per-Machine SQLite
databases — Overdrive's per-instance journal already has that shape.)

### I-3. COPY: feature-flagged VMM version rollout

Fly's v1.7.0 post treats "we can feature-flag Firecracker release rollouts" as a deliverable in its
own right, after years stuck on a pinned old version. Overdrive should design the CH binary path and
version as **config on the `Driver`, per node**, from day one — not a constant. This is cheap now and
expensive later; Fly paid the expensive version.

### I-4. DECIDE DELIBERATELY: block device vs. virtiofs — Fly's evidence does not decide this for you

Fly is all-block-device, but **only because Firecracker has no virtiofs**. Overdrive is targeting
Cloud Hypervisor, which **does** support virtiofs (`virtiofsd`). So this is a genuinely open choice,
and Fly's precedent is not evidence either way.

What Fly's experience *does* show is what you buy by staying at the block layer: LVM2 thin CoW
snapshots, `dm-clone` live migration with lazy hydration, LUKS2 at-rest encryption, `fstrim`/`DISCARD`
as a migration accelerator — an entire ecosystem of block-layer tooling that composes.

What virtiofs would buy Overdrive: **no image-to-block-device conversion at all.** A flattened OCI
layer tree in a host directory + overlayfs + `virtiofsd` is a much shorter path from "we have no image
machinery" to "a VM boots the workload." The costs: a `virtiofsd` process per VM (a supervision and
resource-accounting surface Overdrive would have to own), a larger guest-visible attack surface than
`virtio-blk`, and no `dm-clone`/LUKS2 story.

**Recommendation**: for a single-tenant appliance where live migration is not in scope, **virtiofs +
overlayfs is the cheaper first vertical slice**, and it lets the first CH driver land without building
an LVM thin-pool manager. Revisit block devices when (and only when) snapshot/migration or at-rest
encryption becomes a real requirement. Per CLAUDE.md § "Build vertical slices through production entry
points," the deciding question is which one gets `overdrive serve` + `overdrive deploy <SPEC>` booting
a real microVM soonest — and that is virtiofs.

### I-5. COPY the image-pull design; do not copy containerd

"An OCI image is just a stack of tarballs" is the whole insight. Overdrive needs, in order:

1. A registry client (auth token → manifest list → per-arch manifest → layer blobs by digest).
2. A content-addressed local layer cache keyed by digest — this is the thing that makes the second
   deploy "lightning fast," and it is independent of what you do with the layers afterward.
3. A materialiser: layers → a rootfs the VMM can consume (directory-for-virtiofs, or thin
   device-for-virtio-blk per I-4).

Do **not** pull in containerd. Fly used it in 2021 largely for the devmapper snapshotter; Overdrive is
a single-tenant appliance with an existing `Driver` trait and no need for a second runtime's daemon,
CRI surface, or plugin model. In Rust, `oci-distribution`/`oci-client` + `oci-spec` cover steps 1–2.
**Note this is a recommendation, not a Fly-sourced finding.**

### I-6. COPY: `pilot`'s lesson — write the host↔guest contract before writing the agent

JP Phillips' retrospective — "there really wasn't any contract between `flyd` and `init`" — is a
warning aimed squarely at where Overdrive is about to be. The corrective, stated by Fly themselves, is
to make the in-guest component a runtime with a **defined API** the supervisor drives.

Concretely, per `.claude/rules/development.md` § "Trait definitions specify behavior, not just
signature": the vsock protocol between `overdrive-init` and the host is a **port trait with a written
behavioural contract** — preconditions, postconditions, edge cases (guest never connects; guest
connects then dies mid-RPC; version skew between an old init in a snapshot and a new host), and
observable invariants. Version the protocol from message one. A snapshotted VM carries an old init
forever; skew is not hypothetical.

Specifically, the contract must pin:
- **Readiness**: what exactly does "the workload is up" mean, and who says it? Fly's answer is
  ambiguous in public (init-over-vsock vs. proxy health checks). Overdrive should make init's
  post-`exec` signal authoritative for `Running`, and keep dataplane health checks as a *separate*
  observation.
- **Exit-code reporting**: init must report the entrypoint's exit status and signal over vsock. This
  is the microVM analogue of the exec driver's `ExitObserver` and must feed the same restart-policy
  inputs (`attempts`, `last_failure_seen_at`).
- **Log shipping**: decide explicitly. Console-over-serial is the zero-dependency option and is
  available before networking; vsock is structured but needs the protocol. Fly has not published
  which it uses.

### I-7. AVOID: encoding host identity into the workload's address

Fly's 6PN embeds the host ID in the IPv6 address, and "Making Machines Move" documents the bill:
migration required either address mappings or "burning several weeks doing the direct configuration
fix fleet-wide." Overdrive's per-workload `/30` `workload_addr` is host-local for the same structural
reason. The mitigation Fly converged on is the one Overdrive already chose (ADR-0072): **workloads
dial by name; the address is an implementation detail the dataplane owns.** Hold that line — do not
let a microVM driver leak `workload_addr` into a guest-visible stable identity.

### I-8. AVOID (because Overdrive is single-tenant, not public cloud)

Fly's design is shaped by adversarial multi-tenancy. Overdrive is an appliance. Do **not** port:

- **The jailer.** Fly runs the VMM under an external chroot/namespace/privilege-dropping jailer
  because a tenant may be an attacker. In a single-tenant appliance the VMM is trusted platform code.
  Keep Firecracker/CH's own seccomp filter (free, upstream, no reason to disable it) and Overdrive's
  existing cgroup v2 limits; skip the jailer. Revisit if Overdrive ever runs untrusted third-party
  workloads.
- **Oversubscription / bid-market placement.** Fly's market scheduler and 10x oversubscription exist
  because they sell headroom across a global fleet. Overdrive's placement problem is a single node's
  capacity check.
- **Anycast/BGP edge, WireGuard backhaul, `fly-proxy` TLS termination.** These solve global
  request routing. Overdrive's existing nft-TPROXY + kernel-mediated mTLS layer is the equivalent and
  is already built. Do not grow a second proxy.
- **Suspend-as-scale-to-zero.** Per Conflict 2, suspend does not free capacity. In a single-node
  appliance the only interesting lifecycle verb is stop (frees memory) — VM snapshot/resume is a
  latency optimisation to consider much later, and CH's snapshot support is less mature than
  Firecracker's.

### I-9. The `Driver` trait shape this implies

Fly's Machine state set (`created`/`started`/`stopped`/`suspended`/`destroyed`) draws a line Overdrive's
allocation model currently collapses: **"stopped but rootfs preserved" is a distinct, valuable state.**
It is what makes restart fast (the thin snapshot / rootfs directory survives) and it is what the exec
driver has no analogue for. Before the CH driver is designed, decide whether
`Driver::stop`/`Driver::start` on a microVM preserve the rootfs, and make that explicit in the trait's
docstring — it is exactly the kind of edge case § "Trait definitions specify behavior" exists to force.

**Caveat on the whole section**: I-4, I-5, I-8, and I-9 are **analysis and recommendation**, not
findings sourced to Fly. They are labelled as such. I-1, I-2, I-3, I-6, I-7 are each grounded in a
cited Fly primary source above.

## Full Citations

[1] Fly.io. "Carving The Scheduler Out Of Our Orchestrator". The Fly Blog. 2023-02-01. https://fly.io/blog/carving-the-scheduler-out-of-our-orchestrator/. Accessed 2026-08-01.
[2] Ptacek, Thomas. "Docker Without Docker". The Fly Blog. 2021-04-08. https://fly.io/blog/docker-without-docker/. Accessed 2026-08-01.
[3] Ptacek, Thomas. "Sandboxing and Workload Isolation". The Fly Blog. 2020-07-29. https://fly.io/blog/sandboxing-and-workload-isolation/. Accessed 2026-08-01.
[4] Jordan, Will. "The Serverless Server". The Fly Blog. 2022-06-30. https://fly.io/blog/the-serverless-server/. Accessed 2026-08-01.
[5] Ptacek, Thomas. "Incoming! 6PN Private Networks". The Fly Blog. 2020-12-08. https://fly.io/blog/incoming-6pn-private-networks/. Accessed 2026-08-01.
[6] Ptacek, Thomas. "A Foolish Consistency". The Fly Blog. 2022-03-29. https://fly.io/blog/a-foolish-consistency/. Accessed 2026-08-01.
[7] Ptacek, Thomas. "Making Machines Move". The Fly Blog. 2024-07-30. https://fly.io/blog/machine-migrations/. Accessed 2026-08-01.
[8] Fly.io. "The Exit Interview: JP Phillips". The Fly Blog. 2025-02-12. https://fly.io/blog/the-exit-interview-jp/. Accessed 2026-08-01.
[9] Fly.io. "init-snapshot — README". GitHub (superfly/init-snapshot, branch `public`). https://github.com/superfly/init-snapshot/blob/public/README.md. Accessed 2026-08-01.
[10] Fly.io. "superfly/firecracker". GitHub. Tag `v0.24.6-fly`. https://github.com/superfly/firecracker. Accessed 2026-08-01.
[11] Fly.io. "Machine Suspend and Resume". Fly Docs. https://fly.io/docs/reference/suspend-resume/. Accessed 2026-08-01.
[12] Fly.io. "Machine states and lifecycle". Fly Docs. https://fly.io/docs/machines/machine-states/. Accessed 2026-08-01.
[13] Fly.io. "The Fly.io Architecture". Fly Docs. https://fly.io/docs/reference/architecture/. Accessed 2026-08-01.
[14] Fly.io. "Fly Volumes overview". Fly Docs. https://fly.io/docs/volumes/overview/. Accessed 2026-08-01.
[15] Fly.io. "Persistent Storage and Fast Remote Builds". The Fly Blog. https://fly.io/blog/persistent-storage-and-fast-remote-builds/. Accessed 2026-08-01.
[16] Mackey, Kurt. "Reliability, it's not great". Fly.io Community. 2023-03-06. https://community.fly.io/t/reliability-its-not-great/11253. Accessed 2026-08-01.
[17] akshit-fly. "We shipped Firecracker v1.7.0". Fly.io Community (Fresh Produce). 2024-05-31. https://community.fly.io/t/we-shipped-firecracker-v1-7-0/20140. Accessed 2026-08-01.
[18] Fly.io. "Updated default kernel version". Fly.io Community (Fresh Produce). 2023-06. https://community.fly.io/t/updated-default-kernel-version/13786. Accessed 2026-08-01.
[19] Fly.io. "Custom kernel with Fly Machines?". Fly.io Community (wishlist). https://community.fly.io/t/custom-kernel-with-fly-machines/6082. Accessed 2026-08-01.
[20] Fly.io. "More reliable Machine resumes". Fly.io Community (Fresh Produce). https://community.fly.io/t/more-reliable-machine-resumes/26007. Accessed 2026-08-01.
[21] Fly.io. "Autosuspend is here! (+ Machine suspension is enabled everywhere)". Fly.io Community. 2024. https://community.fly.io/t/autosuspend-is-here-machine-suspension-is-enabled-everywhere/20942. Accessed 2026-08-01.
[22] Fly.io. "Security". Fly Docs. https://fly.io/docs/security/. Accessed 2026-08-01.
[23] Fly.io. "Fly.io security practices and compliance". Fly Docs. https://fly.io/docs/security/security-at-fly-io/. Accessed 2026-08-01.
[24] Fly.io. "Private Networking". Fly Docs. https://fly.io/docs/networking/private-networking/. Accessed 2026-08-01.
[25] firecracker-microvm. "Creating Custom rootfs and kernel Images". GitHub. https://github.com/firecracker-microvm/firecracker/blob/main/docs/rootfs-and-kernel-setup.md. Accessed 2026-08-01.
[26] firecracker-microvm. "Network Setup" / "Design". GitHub. https://github.com/firecracker-microvm/firecracker/blob/main/docs/network-setup.md , https://github.com/firecracker-microvm/firecracker/blob/main/docs/design.md. Accessed 2026-08-01.
[27] containerd. "Devmapper snapshotter". GitHub. https://github.com/containerd/containerd/blob/main/docs/snapshotters/devmapper.md. Accessed 2026-08-01.
[28] Hacker News. Comment 38661624 ("Fly Volumes are attached NVME storage; they're anchored to the physical host"). 2023-12. https://news.ycombinator.com/item?id=38661624. Accessed 2026-08-01.
[29] Clark, Tim Anderson. "Fly.io CEO says reliability 'not great' as platform suffers scaling issues". DevClass. 2023-03-07. https://devclass.com/2023/03/07/fly-io-ceo-says-reliability-not-great-as-platform-suffers-scaling-issues/. Accessed 2026-08-01.
[30] Fly.io. "What Is a Firecracker VM?". Fly Learn. https://fly.io/learn/firecracker-vm/. Accessed 2026-08-01.

## Research Metadata

Sources examined: ~35 · Cited: 30 · Cross-referenced findings: 14 of 24 · Confidence distribution:
High ~58%, Medium/Medium-High ~29%, Low/UNVERIFIED ~13% · Output:
`docs/research/platform/fly-io-microvm-implementation-research.md`

**Tool failures**: none blocking. `fly.io/learn/firecracker-vm/` returned a marketing-oriented page
with less technical detail than expected; the 2020–2024 blog posts carried the substance.
