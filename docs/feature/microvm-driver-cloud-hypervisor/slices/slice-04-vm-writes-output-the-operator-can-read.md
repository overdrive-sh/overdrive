# Slice 04 — A VM job writes output the operator can read, and its storage daemon dies honestly

> DISCUSS brief (2026-08-02 — **new slice**, added when intake decision **`I-6`** was
> recovered). Feature: `microvm-driver-cloud-hypervisor` (GH #42).
> Stories: **US-VM-8**, **US-VM-9**. Job: **J-OPS-003**. Governed by feature-delta `[D8]`.
> Gated by Slice 01 (a VM must run), Slice 02 (the failure vocabulary this extends),
> Slice 03 (`[D3]`'s classification and the bounded-grace shutdown shape), and **Slice 00
> P6**.
>
> **This slice took the `04` slot; the resources slice was renumbered to `05`.** See
> `slice-05-resources-size-the-vm.md`.

## Why this slice exists

Intake **`I-6`**, recovered 2026-08-02. The user's opening message named two decisions the
reference implementation had already made — *"speak with CH over unix socket and use
virtiofsd for storage."* The first became `I-2`; **the second was dropped by the intake
author and never recorded.** Research then recommended `ext4` + `virtio-blk`, the first
DISCUSS pass scoped that, and the reversal was never surfaced.

**The apparent conflict was an artifact of scoping.** DISCUSS scoped the **rootfs** and
never scoped **volumes at all**, so virtiofs fell out by omission rather than by decision.
The research and the reference agree once the roles are separated — `architecture.md:196`:
*"Other drives (code.ext4, deps.ext4) continue as block devices in the `disks` VmConfig
section"*, with only `attach_drive("output", …)` routed to virtiofs. `[D8]` is that split,
written down.

**What it buys, in one line:** a `[vm]` + `[job]` workload that cannot write output
anywhere is not a batch workload — it is a number. Slice 01's per-launch rootfs copy is
discarded at terminal *by design* (that is what makes restart honest), so without a volume
there is **no** path for a guest's bytes to reach the host.

## Goal (one line)

`overdrive deploy render.toml` with a `[[vm.volume]]` block runs a guest that writes
`/output/frame-0120.exr`, and after the job terminates Ana reads that file on the host with
`ls` — **and if the storage daemon dies at any point, `workload describe` says so instead
of reporting a clean run over a truncated file.**

## Learning hypothesis

virtiofs is the right mechanism for the **volume** role on Cloud Hypervisor even though it
is the wrong one for the **rootfs** role — because the argument against a virtiofs rootfs
(no DAX on CH, so FUSE sits in the boot-read hot path for no page-cache-sharing benefit) is
an argument about *that role*, not about the mechanism.

**Predicted:** a guest write round-trips byte-identically to the operator's host directory;
a VM declaring no volume is byte-for-byte the workload Slice 01 shipped; and the daemon's
supervision reuses `VmDriver::start`'s placement and Slice 03's shutdown shape rather than
introducing a parallel process model.

## Thinnest `serve` + `deploy` loop

`overdrive serve` + `overdrive deploy` with:

```toml
[job]
name = "batch-render"

[vm]
kernel  = "/var/lib/overdrive/artifacts/vmlinux-6.18"
rootfs  = "/var/lib/overdrive/artifacts/render.ext4"
command = "/usr/bin/render"
args    = ["--frames", "120", "--out", "/output"]

[[vm.volume]]
source    = "/var/lib/overdrive/outputs/batch-render"
target    = "/output"

[resources]
cpu_milli    = 1000
memory_bytes = 1073741824
```

Then, on the host: `ls /var/lib/overdrive/outputs/batch-render` → `frame-0120.exr`.

Plus seven failure/edge cases: a read-only volume; a spec with **no** volume (the regression
guard); a missing source directory; a missing storage daemon; **a volume the guest cannot
mount** (the composite-lie case); a daemon killed mid-run; and a host that cannot supply
`--sandbox=namespace`.

## Behavior (DESIGN owns the API shape)

- **Spec (`[D8a]`):** `[[vm.volume]]` array in `[vm]` — `source` (host path, required),
  `target` (guest mount point, required), `read_only` (optional, default `false`). All
  three are **operator** surface; the socket path, tag, `--cache` and `--memory shared=…`
  are **platform**.
  > **Why `target` is operator surface while `[D1]`'s `cmdline` is not:** a wrong `cmdline`
  > bricks the boot *undiagnosably* (it looks identical to a corrupt rootfs), whereas a
  > wrong `target` fails *visibly* — the workload writes elsewhere and the host directory
  > is empty. Different diagnosability, different surface call.
- **Mechanism (`[D8]`):** one supervised `virtiofsd` per volume-carrying VM over a
  vhost-user socket. **The rootfs stays `virtio-blk` (`[D5]`)** — the split is by role.
- **`--memory shared=on` is CONDITIONAL (`[D8b]`)** — derived from `!volumes.is_empty()`,
  one field on the `VmConfig` **value** (system constraint 4 already forbids a builder, so
  this is one branch at one construction site, not two config shapes). A volume-free VM
  boots byte-identically to Slice 01.
  **`superseded-by-DESIGN` (2026-08-11, GH #42) — two corrections.** **C-6:**
  `RLIMIT_FSIZE` must be **`max(rootfs image, guest RAM)`** whenever `shared=on` is in
  play, because `shared=on` backs guest RAM with a **memfd** and a memfd is a *file* for
  `RLIMIT_FSIZE` — a limit sized off the rootfs alone kills every volume-carrying VM with
  an opaque `SIGXFSZ`. It is encoded from **Slice 01** (`VmConfig::rlimit_fsize()`,
  ADR-0082 § D2), *before* this slice turns `shared=on` on, so this slice inherits it
  rather than deriving it. **Assumption A-3 (labelled by the system designer,
  2026-08-11):** P6 measured the `shared=on` volume path on **x86_64 only**;
  `findings.md`'s verdict table records *"aarch64 still unmeasured"*. This slice designs
  the volume path for **both** shipping arches on a single-arch measurement — **if
  `shared=on` misbehaves on Arm metal, this slice is x86_64-only until measured.** The
  volume capability is what gates, not the driver.
- **Host tuning prerequisite — `shmem_enabled=advise`, undeferred D-5 (2026-08-11, GH #42).**
  Spike P11 (`spike/findings.md`) measured the host default
  `/sys/kernel/mm/transparent_hugepage/shmem_enabled=never` at **~55% lower durable
  throughput on every `--memory shared=on` path** — exactly the memory backing `[D8b]`
  turns on for a volume-carrying VM. This was first surfaced as deferral **D-5** and filed
  as out-of-scope on the reasoning *"only bites once volumes land (Slice 04)"* — but Slice
  04 **is** this feature, so a prerequisite for it is a prerequisite of the feature, not a
  deferral. Three decisions:
  - **Who sets it — node bootstrap, not the driver, not "the appliance image."** Overdrive
    ships no Image Factory / appliance-image build pipeline yet (intake **I-3**: *"zero
    image machinery today"*); naming that pipeline as the owner would document behaviour
    that does not exist (`.claude/rules/development.md` § "Documentation" — no aspirational
    docs). `shmem_enabled` is one host-wide sysfs knob
    (`/sys/kernel/mm/transparent_hugepage/shmem_enabled`), not per-VM state, so it is set
    **once per node** — never from `Vmm::create`/the driver on every launch, which would
    race concurrent VM starts over one global file and push a host-policy concern into
    per-workload driver code. The landing site is `infra/provision/common-system.sh` (the
    shared Lima + bare-metal provisioning surface both targets already invoke), installing a
    **boot-persistent** write — a `systemd-tmpfiles.d` `w` line or a small oneshot unit,
    mirroring the existing `/etc/sysctl.d/99-overdrive.conf` (`net.ipv4.ip_forward`) and
    `/etc/udev/rules.d/99-kvm4all.rules` (`/dev/kvm`) precedents in
    `infra/lima/overdrive-dev.yaml`. `shmem_enabled` sits under `/sys/kernel/mm/`, **not**
    `/proc/sys`, so the project's `sysctl.d` mechanism does not reach it and a dedicated
    boot-time writer is needed. (When an Image Factory eventually ships, this same
    idempotent write relocates into the image's own boot unit set — the decision is
    unchanged, only where the script that performs it lives.)
  - **What happens when it's `never` — WARN, never refuse.** This is a throughput knob, not
    a correctness or security claim. Unlike `[D8d]`'s `--sandbox=namespace` — whose silent
    downgrade is a *lie about the isolation delivered*, and therefore fails closed — a
    degraded `shmem_enabled` changes nothing about durability or correctness (P11: `fsync`
    is untouched; only the write phase slows). Per **SD-5**'s dichotomy, *"a substrate lie
    refuses the node, a capability absence does not"* — this is neither: a suboptimal but
    honest host configuration. Refusing to boot a volume-carrying VM over it would leave the
    operator strictly worse off than the slow-but-correct VM they would otherwise get. The
    check extends the **existing** `Vmm::probe()` boot gate (Slice 01; SD-5 Reuse Analysis
    row 3) with one more read: `shmem_enabled` is a host fact that "cannot change between
    deploys on the same node" (SD-5's own reasoning for the CH-absence refinement applies
    verbatim here), so the check fires **once at node boot**, not per-VM-launch, and never
    gates `[vm]` admission.
  - **Where it's observable.** A structured `tracing::warn!(name: "health.startup.warn",
    reason: "vmm.shmem_enabled_suboptimal", observed: "never", recommended: "advise")` at
    boot, naming the measured cost (P11: ~55% lower durable `shared=on` throughput) and the
    fix (the provisioning script above) — the same `health.startup.{refused,warn}`
    vocabulary every other Earned-Trust probe in this codebase uses
    (`crates/overdrive-dataplane/src/allocators/vip_range.rs:256` is the one prior reference
    to `health.startup.warn`; this is its first real emit site). This is what turns a 55%
    throughput regression into an attributable boot-time fact instead of a mysterious
    volume-performance complaint days later.
- **`--cache=never` (`[D8c]`)** — exactly one guest mounts each share, and CH has no DAX, so
  a guest page cache over the share would be plain double-buffering.
- **`--sandbox=namespace`, fail-closed (`[D8d]`)** — virtiofsd's own default, giving *that*
  process the mount namespace + `pivot_root` that `[D7]` could not give the hypervisor. A
  host that cannot supply it lands `Failed`. **It never degrades to `chroot`** — which is
  verbatim what the reference implementation silently did, with no rationale recorded
  anywhere (intake precedent warning #6's shape).
- **Placement (`[D8e]`):** the daemon sits in the allocation's cgroup scope (load-bearing —
  `cgroup_kill` must reap it) and its netns. **The volume `source` directory is NOT added to
  the hypervisor's Landlock ruleset** — CH touches only the socket; only `virtiofsd` reaches
  the data. Volumes therefore do not widen `[D7]`.
- **Guest side (`[D4]` amendment, `[D8g]`) — scoped, not passive.** `overdrive-init` gains a
  fifth duty: **mount each declared volume at its `target` before exec'ing the command, and
  refuse to exec if a required mount fails.** A secondary virtiofs share is *not*
  auto-mounted by the guest kernel, and PID 1 is the only in-guest process Overdrive
  controls. **This expands a locked decision and is recorded as an amendment**, because the
  first draft wrote *"and mounts the share in the guest at `target`"* in the passive voice
  with no subject — which is exactly how `I-6` was lost in the first place. The
  BYO-artifact contract extends with it: the operator's kernel must provide virtiofs
  support.
- **`read_only` is enforced HOST-side (`[D8g]`).** A guest-side `-o ro` is applied too, but
  only as an ergonomic guard — it is guest-cooperative and would be void against exactly the
  untrusted workload `[D7]` is written for, so it is **not** the boundary and no artifact may
  call it one.
- **Ordering (US-VM-9):** daemon running **and socket ready** before `vm.create`; VM torn
  down before the daemon; no leaked socket or orphan on any intermediate failure path. A
  readiness wait that expires is its own named `Failed` reason.
- **Exit honesty (US-VM-9, `[D3]` + system constraint 9):** classification uses the
  **workload's** reported outcome, never the daemon's exit status — in **either** direction.
- **Failure vocabulary:** four new `TransitionReason` variants minted in the **Slice 02
  shape** — volume-source-not-found, storage-daemon-absent, **guest-mount-failed**, and
  socket-readiness-timeout — plus sandbox-unavailable. None collides with Slice 02's four or
  US-VM-7's fifth.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes, and unusually literally: the assertion is
  an **ordinary filesystem read on the host**, outside Overdrive entirely. There is no way
  to fake it through a test seam.
- **Thinnest?** Yes for what it delivers, and it was **not** folded elsewhere: Slice 01 is
  the over-budget walking skeleton, Slice 03's subject is the *hypervisor* process, and
  Slice 05 is 2 days of derivation logic. None is a home for a second supervised process.
- **Delivers operator-visible value alone?** Yes — this is what makes the VM class
  *productive* rather than merely trustworthy.
- **Every story operator-visible?** Yes, both. US-VM-8 puts a file in Ana's directory;
  US-VM-9 decides what `workload describe` says about the run that produced it. The
  slice-composition gate holds without needing an argument.
- **Ships a lie?** No — **and this is why US-VM-8 and US-VM-9 must land together.** A slice
  that ships working volumes while a `virtiofsd` death is misclassified is a slice whose
  deliverable is a lie, which is the same test that rejected Slice 01's three candidate
  sub-splits. Two stories, one slice, one landing.
- **Guards against the most likely lie?** Yes. The reference implementation got the
  sidecar classification wrong in **both** directions at once — a clean `virtiofsd` exit read
  as `VmmError::Crash("virtiofsd crashed unexpectedly")` that force-killed the VM, and a
  mid-run death that never reached `ExitKind` at all. Both arms are **mandatory
  mutation-testing targets** (K9).

## Acceptance (= US-VM-8 + US-VM-9 ACs)

### US-VM-8 — the capability

- [ ] A guest write inside a declared volume is present, readable and **byte-identical** in
      the operator's host `source` directory after terminal — Tier-3, through a real
      `serve` + `deploy`, with the host-side read done by ordinary filesystem access and
      **not** through any Overdrive API.
- [ ] `read_only = true` is enforced **host-side** (`[D8g]`): the guest write fails and the
      host directory is byte-unchanged. **Observed from inside the guest and on the host**,
      not asserted against the flag the platform passed — and **the case must defeat a
      guest-side-only implementation**, since a cooperative guest's write failing proves
      nothing about an uncooperative one.
- [ ] **A failed required mount is `Failed`, never a completed run.** A volume the guest
      cannot mount yields its own named reason, the operator's command is **never executed**,
      and the allocation **never reaches a completed terminal state**. **The composite-lie
      case**: without this, the command writes into the discarded per-launch rootfs copy at
      `target`, exits 0, and `workload describe` reports
      `Terminated / Completed{exit_code: 0}` over an **empty** host directory — every
      individual signal truthful, the composite false.
- [ ] **Regression guard.** A spec declaring **no** volume starts **no** storage daemon,
      derives `shared=on` **off**, and reaches the same terminal state and exit code as it
      did before this slice. Slices 01–03 must not move.
- [ ] Volume-source-not-found and storage-daemon-absent are **two distinct**
      `TransitionReason` variants in the Slice 02 shape; neither collides with that slice's
      four nor with US-VM-7's fifth. Each names the resource and the next step, verified by
      reading `workload describe` output rather than by asserting on an enum.
- [ ] **`[D8e]` non-widening.** The volume `source` directory does **not** appear in the
      live `cloud-hypervisor` process's Landlock ruleset; the hypervisor's reach is
      unchanged from US-VM-7. **And the rest of the `[D7]` posture holds unchanged for a
      volume-carrying allocation** (`/proc/<vmm-pid>/{status,limits,cgroup,ns/net}`), so K7
      covers volume-carrying VMs and not only volume-free ones — one extra assertion on an
      allocation this slice already boots.
- [ ] **Engineering constraint (binding, not UAT-derived):** the only keys accepted under
      `[[vm.volume]]` are `source`, `target` and `read_only`; any other key is **rejected by
      name**. Stated as the observable rather than as "`--cache` is platform-derived",
      because a construction property over code that never names the flag has no test and no
      mutation site.
- [ ] **Engineering constraint (binding, not UAT-derived; undeferred D-5):** node boot emits
      `health.startup.warn` (`reason: "vmm.shmem_enabled_suboptimal"`) exactly once, at
      `Vmm::probe()` time, when `/sys/kernel/mm/transparent_hugepage/shmem_enabled != advise`
      — and **never** gates `[vm]` admission on it. Verified by reading the emitted event,
      not by asserting on throughput (the throughput cost itself is P11's measurement, not a
      re-derived AC here).

### US-VM-9 — the honesty

> **These three ACs are ONE discriminated classification and must not be implemented as
> three independent checks.** The discriminator is a single guard — *did the daemon's exit
> arrive **before** the workload reported an outcome, or during teardown?* The first AC is
> **vacuous without it**: a do-nothing implementation that ignores the daemon also produces
> `Terminated / Completed`, and cargo-mutants cannot *insert* a guard the code does not
> contain. It is non-vacuous only because the second AC's daemon watcher forces that guard
> to exist. Same trap already closed twice in this feature — US-VM-1 item 6 (argv assertion)
> and US-VM-7 item 1 (named executors).

- [ ] **A clean storage-daemon exit is never a crash.** A VM job completing normally reaches
      `Terminated / Completed{exit_code: N}`; the daemon's own exit contributes nothing.
      **The Tier-3 case must discriminate rather than observe a green result: assert the
      daemon's exit WAS observed** (it appears in the allocation's event/audit trail)
      **while contributing nothing to `ExitKind`** — an observation a do-nothing
      implementation cannot produce. **Mutation target: removing or negating the
      before-vs-during-teardown guard must be killed by this case.**
- [ ] **A mid-run storage-daemon death is never a clean exit.** It yields `Failed` with a
      distinct reason naming the storage daemon — Tier-3 case that kills the daemon while
      the guest runs. **Mutation target: collapsing that arm into `CleanExit` must be
      killed.** This is the arm that makes the guard exist.
- [ ] No code path derives `ExitKind` from the daemon's exit status, in **either** direction
      (system constraint 9).
- [ ] **Ordering is enforced, not assumed.** The daemon is running *and its socket is ready*
      before `vm.create`; the VM is torn down before the daemon. An expired readiness wait is
      its own named `Failed` reason.
- [ ] **`--sandbox=namespace` or nothing.** A host that cannot supply it lands `Failed` with
      a named reason; **no code path starts the daemon with a weaker sandbox.** Mutation
      target: turning the fail-closed arm into downgrade-and-continue must be killed. The
      unavailable condition is **injected at the `Vmm` port boundary** (system constraint 1
      permits a `Sim*` adapter at a port), because the whole test envelope runs on one Lima
      kernel.
- [ ] **No leak on any path** — including a failed start, a readiness timeout and a sandbox
      refusal: no daemon process in the allocation's cgroup scope, no vhost-user socket on
      disk.
- [ ] Every case driven through a real `overdrive serve` + `overdrive deploy`, never a
      test-invoked driver method.

## Size — 6–9 d, re-budgeted after peer review

The reference's `VirtiofsdManager` was **415 lines**. The first pass sized this slice at
4–6 d having priced only the **host-side daemon lifecycle**; peer review found five omitted
rows. The corrected `[D8f]` table:

| Concern | Reused from | New | Est. |
|---|---|---|---|
| Supervised host process; cgroup + netns placement | `VmDriver::start` (Slice 01) | — | 0 |
| `SIGTERM` → grace → `SIGKILL` | Slice 03's bounded-grace shutdown | — | 0 |
| Allocation-scoped socket path | `[D1]`'s platform-derived CH API socket path | — | 0 |
| No-leak-on-terminal hygiene | Slice 03 + the cgroup-leak discipline | — | 0 |
| Ordering: socket-ready before `vm.create`, teardown after the VM, no leak in between (incl. the readiness timeout) | — | **Yes** | 2 d |
| Two-directional exit honesty | `[D3]` states the rule | **Yes — the classification** | 1 d |
| `[[vm.volume]]` array-of-tables parse + validation *(omitted first pass)* | the `[vm]` parse surface | **Yes** | 0.5 d |
| `VmConfig` volume payload + derived `shared` flag *(omitted)* | the `VmConfig` value shape | **Yes** | 0.5 d |
| **Guest-side mount + refuse-to-exec + host→guest tag/target protocol** *(omitted — the critical one)* | the agent's vsock framing | **Yes** | 1 d |
| Host-side `read_only` export enforcement *(omitted)* | — | **Yes** | 0.5 d |
| Failure vocabulary — four new variants | Slice 02's shape | **Yes** | 0.5 d |
| Tier-3 harness — round-trip, read-only, mid-run kill, teardown, no-volume regression, mount failure | Slice 03's harness | **Yes** | 1.5 d |
| A possible **second** rkyv envelope bump if `[[vm.volume]]` reaches the persisted aggregate | `[G4]`'s procedure | **Conditional** | 0–0.5 d |
| `shmem_enabled` boot-time WARN check *(undeferred D-5, 2026-08-11)* | `Vmm::probe()`'s existing gate (Slice 01) | **Yes — one read + one warn event + the `infra/provision/common-system.sh` write** | 0.5 d *(corrected 2026-08-11 — was 0 d, see note below)* |

US-VM-8 ≈ 3.5–5 d, US-VM-9 ≈ 2.5–4 d. **The largest slice after the walking skeleton, whose
upper bound it now meets.**

> **Correction, 2026-08-11 (Morgan, sanity-checked while re-sizing Slice 01).** The
> `shmem_enabled` row above was **0 d** on the reasoning that it fully piggybacks on
> Slice 01's `Vmm::probe()`. That undersold two real, if small, pieces of work its own
> "New: Yes" already concedes exist: the probe-side sysfs read + comparison + structured
> `tracing::warn!` emission (unlike every genuine `0 d` row above, which reuses code
> **verbatim** with no new line written), and the `infra/provision/common-system.sh`
> boot-persistent write this slice's own Behavior/Dependencies sections assign here, not to
> Slice 01 or to a nonexistent Image Factory. Corrected to **0.5 d**. **This does not move
> the 6–9 d headline band** (the table's raw sum was already comfortably under 9 d) **and
> does not trip guardrail 2** (Slice 04's own >9 d lift trigger, re-run at the DESIGN
> handoff and confirmed still not firing).

> **Lift trigger, numbered:** if this slice exceeds **9 days** — its stated upper band —
> lifting it into its own feature is **pre-authorised** (feature-delta § Scope assessment,
> guardrail 2). Nothing depends on it and Slice 05 is independent, so lifting needs no
> restructuring. **DESIGN owns the re-run at the DESIGN handoff and returns a blocker if it
> fires.** Consequence if lifted, stated so it is not glossed: this feature's honest
> deliverable becomes a VM class that computes but cannot deliver, and US-VM-8/9 become a
> hard prerequisite before `[vm]` + `[job]` is presented to operators as production-ready.

## Dependencies

- **Slice 00 P6** — `virtiofsd` + `--memory shared=on` compose with a real boot **and** with
  the `[D7]` confinement flags; the `shared=on` cost measured; the CH ruleset path list
  confirmed (specifically that the volume `source` is **not** in it); **whether host-side
  read-only export works** (if not, `[D8a]`'s security framing is struck before this slice is
  built); **whether the guest can mount the share, and what a failed mount looks like from
  inside** (the evidence the refuse-to-exec path is built against); and the measured volume
  I/O cost under `--cache=never`. **If P6 came back DOESN'T-WORK, `[D8]` is re-decided before
  this slice is built** — the claim follows the code, never the reverse.
- **Slice 01** — a VM must boot before it can mount anything.
- **Slice 02** — this slice extends that failure vocabulary rather than minting a parallel
  one.
- **Slice 03** — `[D3]`'s classification and the bounded-grace shutdown shape.
- **Open DESIGN input:** whether `[[vm.volume]]` reaches the persisted `Job` aggregate, and
  therefore whether it rides inside Slice 01's `JobEnvelope` **V2** or needs a later bump.
  `[G4]`'s six-step single-commit procedure governs either way and existing golden fixtures
  are never touched — **settle it before this slice is built, not inside it.**
- **Independent of Slice 05.** The 04-before-05 ordering is a priority call, not a
  dependency; the only thing lost by swapping them is US-VM-5's both-memory-shapes sizing
  case.
- **Node-bootstrap prerequisite, undeferred D-5 (Slice 00 P11, `spike/findings.md`).**
  `infra/provision/common-system.sh` must set `shmem_enabled=advise` (boot-persistent, see
  Behavior above) before this slice's throughput-sensitive ACs are measured for real —
  otherwise Tier-3 evidence for the volume path is silently captured against the ~55%-
  penalized host default and any future re-measurement is not comparing like with like.

## Explicitly NOT in this slice

`overdrive-fs` chunk store, content-addressed volumes, volume lifecycle independent of the
allocation, multi-VM shared volumes, `vhost-user-blk`, and `--cache` as operator surface.
Persistent-microVM storage is GH
[#97](https://github.com/overdrive-sh/overdrive/issues/97); this is a host-directory share
scoped to one allocation.

**Also not here: `virtiofsd`'s deeper security posture** — its seccomp filter set, which uid
it runs as, its xattr/ACL surface, and its own guest→daemon→host threat model are GH
[#258](https://github.com/overdrive-sh/overdrive/issues/258), **unconditionally** as of the
2026-08-02 amendment. This slice owns the daemon's **lifecycle** and the `--sandbox`
*selection* + fail-closed rule (a spawn-argument property of the launch it performs) — the
same boundary by which `[D7]` item 6 owns "seccomp never weakened" while the rest of the
hypervisor's seccomp posture is #258's. See `[D7]`'s boundary table.
