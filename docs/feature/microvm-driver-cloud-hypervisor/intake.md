# Intake — `microvm-driver-cloud-hypervisor`

Captured at `/nw-new` time (2026-08-01), before DISCUSS. These are the
user's answers to the wizard's scoping questions plus the grounding scan
that produced them. DISCUSS must treat these as inputs, not as settled
design — but must not silently reverse them.

## Goal

Ship a Cloud Hypervisor microVM `Driver` for Overdrive: a workload
declared `[microvm]` in a TOML spec boots as a real CH VM through
`overdrive serve` + `overdrive deploy`.

## Decisions taken at intake

| # | Decision | Rationale |
|---|---|---|
| I-1 | **Reference implementation is shape-only.** `/Users/marcus/conductor/workspaces/opencapsule/algiers` is read for structure; no code is lifted. | The algiers CH driver is a feature-gated prototype that is never instantiated by any production path — 2 API endpoints (`vm.create`, `vm.boot`), no networking, SIGKILL-only shutdown, no boot detection, no response deserialisation, and a compile break in its own test module (`parse_ch_version` / `validate_ch_version` called but never defined). Its roadmap step 03-01 (wire the factory into the runner) was never executed. |
| I-2 | **A `Vmm` port trait sits under the `Driver`.** `MicroVmDriver: Driver` composes over `Arc<dyn Vmm>`; `CloudHypervisorVmm` is the production adapter. | User direction, and independently required — see "Why the VMM trait is load-bearing" below. |
| I-3 | **First slice is BYO-artifact.** The `[microvm]` spec points at a prebuilt kernel + rootfs already on the host. No OCI client, no image factory, no registry pull in slice 1. | Overdrive has **zero** image machinery today (no OCI client, no content-addressed blob store, no rootfs assembly). Bundling the image factory into this feature makes it undeployable for months. Per CLAUDE.md § "Build vertical slices through production entry points": thinner-but-live beats complete-but-dead. |
| I-4 | **Starting wave is DISCUSS.** | Greenfield mechanism, requirements not yet written. |
| I-5 | **One VM driver. TOML table `[vm]`; `DriverType::Vm` survives; `DriverType::MicroVm` is DELETED.** Full VMs are not a supported workload class. | See "Decision I-5" below. |
| I-6 | **Storage splits by role: `virtio-blk` for the ROOTFS, `virtiofs` for VOLUMES / shared writable storage.** | See "Decision I-6" below. **This decision was given at intake and dropped by the intake author; recovered 2026-08-02.** |

### Decision I-6 — virtiofs for volumes, block for rootfs

**Process note, recorded deliberately.** The user's opening message named
two decisions the reference implementation had already made: *"speak with
CH over unix socket and use virtiofsd for storage."* The first was carried
into I-2. **The second was dropped — it appears nowhere in I-1…I-5.** The
research then recommended `ext4` + `virtio-blk` for the rootfs, DISCUSS
scoped that, and the reversal was never surfaced. This file's own preamble
says intake decisions "must not be silently reversed"; that rule was
broken here. The correction below is the user's ruling of 2026-08-02.

**The split:**

| Role | Mechanism |
|---|---|
| **Rootfs** — read-mostly, boots the workload | `ext4` image over **`virtio-blk`** |
| **Volumes / shared writable storage** — the workload's output, persistent state | **`virtiofs`** (`virtiofsd`, `--memory shared=on`) |

**This is not a compromise — it is what the reference implementation
actually shipped.** algiers used block for the rootfs and virtiofs *only*
for the writable `/output` share: *"Other drives (code.ext4, deps.ext4)
continue as block devices in the `disks` VmConfig section"*
(`architecture.md:196`); `attach_drive("output", …)` was the single call
intercepted and routed to virtiofs. The research's recommendation and the
reference's design agree; the apparent conflict was an artifact of DISCUSS
scoping the rootfs and **never scoping volumes at all**, so virtiofs fell
out by omission rather than by decision.

**Why block for the rootfs.** virtiofs **DAX — the mechanism that would
give cross-VM page-cache sharing — does not exist on Cloud Hypervisor.**
CH's own docs: *"Given the DAX feature is not stable yet from a daemon
standpoint, it is not available in Cloud Hypervisor"*; it is also
unimplemented in the Rust virtiofsd. Without DAX, virtiofs on the rootfs
costs a supervised per-VM daemon, forces `--memory shared=on`, and puts
FUSE in the hot path (measured 0.78× native; AWS Lambda is moving away
from FUSE) while delivering none of its headline benefit.

**Why virtiofs for volumes.** It is the *right* mechanism for shared
writable storage and one of the three genuine Cloud Hypervisor
differentiators per whitepaper §6 (with CPU hotplug and Windows guests).
It is also the platform's stated direction — [#97](https://github.com/overdrive-sh/overdrive/issues/97)
plans `vhost-user-fs` for persistent microVMs.

**Correction to an earlier claim in this file's lineage.**
CVE-2026-24834 (CVSS 9.3) was cited during intake as evidence against
virtiofs. That was a **misattribution**: the CVE concerns **`virtio-pmem`
+ DAX**, a different mechanism — the research says so directly
(*"CVE-2026-24834 was about virtio-pmem's read-only enforcement"*). It
argues against a pmem/DAX rootfs. It says nothing about virtiofs.

**Scope consequence.** Volumes are a capability DISCUSS did not scope.
`virtiofsd` is not free — the reference's `VirtiofsdManager` was 415 lines
of socket-wait / SIGTERM→SIGKILL / `Drop` lifecycle, and a crashed
`virtiofsd` must not silently become a "clean" VM exit. Volumes therefore
land as their own slice, not folded into the already-oversized walking
skeleton. Hardening consequence: the virtiofsd surface in
[#258](https://github.com/overdrive-sh/overdrive/issues/258) is
**unconditional**, not "only if we adopt virtiofs".

### Decision I-5 — collapse `MicroVm` + `Vm` into one `vm` driver

**Overdrive will not support full VMs.** There is therefore exactly one
VM-class driver, and the microVM-vs-full-VM distinction that justified two
`DriverType` variants does not exist. Two variants dispatching to identical
code is surface the deferred ADR-0022 registry would have to route between
for no reason.

- **TOML table:** `[vm]`.
- **Surviving variant:** `DriverType::Vm` (`as_str` → `"vm"`,
  `FromStr("vm")`). Table name still equals the tag, so ADR-0031's
  table-name-is-the-discriminator property holds.
- **Deleted:** `DriverType::MicroVm`. `FromStr("microvm")` becomes
  `UnknownDriverType` — nothing constructs or emits it today.
- **Untouched:** `Unikernel` and `Wasm` stay. They are unimplemented but
  genuinely *different* drivers — the Unikraft research establishes that a
  unikernel's guest differences (no `fork()`, no in-guest process tree,
  build-time-fixed process model) are `Driver` **contract** differences,
  not config. That is the principled line: collapse variants that share a
  driver, keep variants that don't.

**Single cut, no compatibility surface.** This is a greenfield project with
no production users, per `feedback_single_cut_greenfield_migrations.md`: no
deprecation, no alias, no grace period, no reserved-for-compat variant.
Delete `MicroVm` and land `[vm]` in the same PR.

**Blast radius — verified, and smaller than the "stable wire form"
docstring implies:**

- **`DriverType` itself: no rkyv envelope bump.** It never reaches a
  persisted row. `AllocStatusRowV1`/`V2`
  (`crates/overdrive-core/src/traits/observation_store.rs:673`, `:741`)
  carry `WorkloadKind`, `AllocState`, `TransitionReason`,
  `TerminalCondition` — no `DriverType`. Its only wire appearance is
  `TransitionSource::Driver(DriverType)`
  (`crates/overdrive-control-plane/src/api.rs:622`), which is serde +
  utoipa on the HTTP/`LifecycleEvent` surface, not rkyv.
- **But `WorkloadDriver` IS rkyv-persisted — and that is a different
  type.** **CORRECTED 2026-08-01 by the DISCUSS wave.** The intent-side
  `WorkloadDriver` enum (`crates/overdrive-core/src/aggregate/mod.rs:162`)
  derives `rkyv::Archive` (`:151-161`) and is archived inside the `Job`
  aggregate behind `JobEnvelope`, with golden-bytes fixtures at
  `crates/overdrive-core/tests/schema_evolution/workload_intent.rs`.
  Adding `WorkloadDriver::Vm` shifts `Job`'s archived layout, so it
  triggers the **six-step single-commit version-bump procedure** in
  `.claude/rules/development.md` § "rkyv schema evolution". Budget it
  inside the walking-skeleton slice; do not discover it there.

  Note the two are independent: deleting `DriverType::MicroVm` is free;
  adding `WorkloadDriver::Vm` is not.

  **RULED 2026-08-02 — do the proper bump.** `JobEnvelope` V1 → V2, the
  full six-step single-commit procedure. The cheaper alternative
  (mutate V1 in place and regenerate its golden fixture) was considered
  and **rejected**: it is affordable only because nobody is in
  production, and it requires *regenerating a golden fixture* — the one
  move `.claude/rules/testing.md` explicitly forbids (*"prior
  `FIXTURE_V_N` literals are NEVER touched"*). The existing V1 fixture
  stays untouched and is reused as the V1→V2 evidence, so the real cost
  is one `From<JobV1> for JobV2` impl plus one new fixture. Greenfield
  buys us the right to skip *migrations*, not the right to erode the
  schema-evolution signal.
- **OpenAPI spec regenerates.** `DriverType` derives `ToSchema` and is
  registered transitively through `TransitionSource`, so the generated
  enum loses a value and `cargo openapi-check` will flag it. Run
  `cargo openapi-gen` in the same commit.
- **Two exhaustive matches lose an arm** —
  `DriverType::as_str` (`traits/driver.rs:56`) and `FromStr`
  (`:76`). Compiler-enforced; no silent drift.
- **Docstring is now wrong.** `traits/driver.rs:26-29` claims *"Stable:
  new drivers are appended; existing variants never change their wire
  form."* Amend it — under greenfield single-cut that sentence is
  aspirational, and CLAUDE.md forbids leaving it standing.

**Vocabulary fallout.** "microvm" is load-bearing in prose across
whitepaper §6 (`:519`, `:607`), issues #96–#100 ("Persistent microVMs"),
#222 ("guest-stack (microVM/unikernel)"), and this feature's own slug
`microvm-driver-cloud-hypervisor` / branch name. The *slug* is fine — it
describes what is being built. The *operator-facing* vocabulary is now
`vm` everywhere. Whitepaper §6 and ADR-0031:539 (*"Future drivers add new
sibling tables (`[microvm]`, `[wasm]`)"*) need amending **through the
architect agent**, not inline.

## Why the VMM trait is load-bearing (I-2)

Not merely a tidy layering. Two project rules force it:

1. **DST blindness.** Everything the CH driver touches beyond the
   existing `Clock` / `CgroupFs` ports — spawning the `cloud-hypervisor`
   process, the HTTP-over-unix API socket, virtiofsd, tap/vsock setup —
   is real host I/O. Without a port trait in `overdrive-core` and a
   `SimVmm` binding in `overdrive-sim`, none of it is reachable from
   Tier-1 DST (`.claude/rules/testing.md` § "Nondeterminism must be
   injectable").
2. **dst-lint.** Those same calls are banned on any `core`-class compile
   path (`xtask/src/dst_lint.rs`). The trait is where the boundary has
   to land regardless.

**The layer split:**

- `Driver` (existing, `overdrive-core`) — allocation-facing: start/stop/
  status/resize, `ExitEvent` emission, the Running-confirmed gate, probe
  hooks. Unchanged.
- `Vmm` (new, `overdrive-core`) — hypervisor-facing: create / boot /
  shutdown / inspect **one VM**. Knows nothing about allocations.
- `MicroVmDriver` (`overdrive-worker`, `adapter-host`) — implements
  `Driver` over `Arc<dyn Vmm>`. Owns the allocation-shaped concerns the
  `ExecDriver` already models: exit watcher, gate ordering, console/log
  tail, cgroup placement of the VMM process, netns entry.
- `CloudHypervisorVmm` (`adapter-host`) — implements `Vmm`.
- `SimVmm` (`overdrive-sim`, `adapter-sim`) — implements `Vmm` for DST.

**Shape caveat for DESIGN — do NOT copy algiers' trait.** Its
`Virtualizer` is a stateful config-accumulating builder (`configure` →
`set_boot_source` → `attach_drive` → `start`) policed at runtime by a
`VmState` enum and two `validate_state_*` functions. That is a hand-rolled
state machine re-checking at runtime what a value type gets for free. The
Overdrive shape should be a `VmConfig` **value** plus a single
`Vmm::create(&VmConfig)`, making "boot before configured" unrepresentable
rather than validated (`.claude/rules/development.md` § "Type-driven
design"). Per CLAUDE.md § "Implement to the design", the exact signature
gets pinned in DESIGN — crafters must not improvise it.

**Cloud Hypervisor is the only `Vmm` implementor in scope.** The trait
would admit a `FirecrackerVmm` as a second adapter later, but that is not
a deliverable here and must not be designed for — a two-implementor trait
built with one implementor is speculative generality. Design the trait to
fit CH honestly; a second adapter can force its own changes when it
actually exists.

## Tracking issue

**GH [#42](https://github.com/overdrive-sh/overdrive/issues/42) — "[3.5]
Cloud Hypervisor microVM + VM driver (unified VMM)"** is this feature's
tracking issue. Verified `gh issue view 42 --comments`: **body is a stub,
zero comments, and its Acceptance section is a literal
`<!-- TODO(assignee): add <=3 acceptance bullets before picking this up -->`
placeholder.** So there is no ratified scope in the tracker — DISCUSS is
writing it, not inheriting it. Its only stated notes are *"Replaces
Firecracker + QEMU default; moved from Phase 4"* and a dependency on #12
(nondeterminism traits — CLOSED).

Downstream issues that depend on #42, and therefore bound what this
feature must not foreclose:

| Issue | Title | Why it matters here |
|---|---|---|
| [#92](https://github.com/overdrive-sh/overdrive/issues/92) | Right-sizing reconciler (cgroup for processes, **CH hotplug for VMs**) | The commercial pillar that makes CPU hotplug load-bearing — see below |
| [#96](https://github.com/overdrive-sh/overdrive/issues/96) | Persistent microVMs 1: CH snapshot/restore + `userfaultfd` + VMGenID | Needs the driver to own a VM long enough to snapshot it |
| [#97](https://github.com/overdrive-sh/overdrive/issues/97) | Persistent microVMs 2: `overdrive-fs` chunk store + vhost-user-fs | The eventual image/storage layer this feature defers |
| [#100](https://github.com/overdrive-sh/overdrive/issues/100) | Persistent microVMs 5: `overdrive-guest-agent` (ttRPC/vsock, SPIFFE) | The guest agent — intake open question 3 |
| [#222](https://github.com/overdrive-sh/overdrive/issues/222) | Guest-stack intercept adapter for transparent mTLS | microVMs terminate TCP in the *guest*; `cgroup_connect4`/sockops are structurally blind. See open question 5 |

**SUPERSEDED 2026-08-02 by user ruling.** This paragraph previously said #42's
acceptance bullets *"should be filled in from DISCUSS output, with user
approval."* **Approval was requested and NOT granted: #42 is left alone.**
`feature-delta.md` is the ratified scope for this feature; #42 remains a
stub tracking issue and nothing downstream should wait on, or write toward,
its `TODO` placeholder. The dependency is dropped outright — no replacement
forward pointer stands in its place.

## The VMM premise — VERIFIED, and narrower than assumed

The stated reason for Cloud Hypervisor is *"memory AND CPU hotplug"*.
Checked upstream on 2026-08-01. **Half of it is a real differentiator;
the other half is not — and the real half is sufficient.**

| Capability | Cloud Hypervisor | Firecracker | Verdict |
|---|---|---|---|
| **CPU hotplug** | ✅ (ACPI) | ❌ — [#2609](https://github.com/firecracker-microvm/firecracker/issues/2609) OPEN, `Priority: Low`, `Status: Parked`, filed 2021-06-03 | **Genuine CH-only differentiator** |
| **Memory hotplug** | ✅ (virtio-mem) | ✅ — **shipped v1.14.0, 2024-12-17** ([PR #5534](https://github.com/firecracker-microvm/firecracker/pull/5534)), documented at `docs/memory-hotplug.md`; plug *and* unplug | **No longer a differentiator** |
| virtiofs | ✅ | ❌ (permanently foreclosed) | CH-only |
| Windows guests | ✅ | ❌ | CH-only |
| AArch64 | ✅ | ✅ (closed since the 2026-04-19 research) | Parity |

Firecracker v1.16.0 added *device* hotplug (PCI virtio block/net) — **not
vCPUs**. Its balloon device (`docs/ballooning.md`) only reclaims within
the boot-time allocation; it cannot grow a VM. Firecracker's memory
hotplug is real virtio-mem, but is **not permitted on snapshot-restored
VMs** — a constraint that interacts with #96.

**Why the surviving half is enough.** CPU hotplug is not a nice-to-have;
it is the mechanism behind GH #92, the right-sizing reconciler, which
whitepaper §14 names as a commercial pillar (the "pre-OOM pressure
signal" the whitepaper claims has *"no published production analogue"*).
Whitepaper:1728 — *"VM and unikernel workloads are right-sized via Cloud
Hypervisor's hotplug APIs — memory via virtio-mem, CPU via ACPI."* On
Firecracker that reconciler could resize memory but never CPU, so half
the right-sizing story dies for VM-class workloads. **State the argument
as "CPU hotplug unblocks #92", not as "CH has hotplug" — the latter is
refutable, the former is not.**

### Whitepaper §6 is internally contradictory — flag, do not cite

- **whitepaper.md:535** — the matrix row `| CPU / memory hotplug | ❌ | ✅ | ✅ |`
  collapses two capabilities into one and marks Firecracker ❌. **Wrong
  for memory since 2024-12-17.**
- **whitepaper.md:601** — the prose immediately below is *correct*:
  *"Firecracker did gain virtio-mem memory hotplug in 2024… CPU hotplug,
  virtiofs, and Windows guest support remain the genuine Cloud Hypervisor
  differentiators."*

The fix is to split row 535 into two rows. Per
`feedback_whitepaper_ssot.md` the whitepaper is **not** SSOT and is known
stale; per CLAUDE.md, design-artifact edits go through the architect
agent. So: **do not cite whitepaper:535 as evidence, and do not edit it
inline.** DESIGN should route the correction through the architect.

Whitepaper:626's *"Cloud Hypervisor exposes a VMGenID device"* claim
remains **UNVERIFIED** (flagged 2026-04-19, never checked). It is a #96
premise, not a this-feature premise — but nobody has grounded it.

### The reference implementation dismissed hotplug — and that is informative

algiers chose CH for **virtiofs `/output` persistence + AArch64**, and
its research doc explicitly rejected hotplug as a motivation:

- `docs/research/cloud-hypervisor-migration-research.md:261` — *"vCPU
  hotplug is irrelevant for OpenCapsule since VMs are created with fixed
  resources and destroyed after job completion."*
- `:362` — hotplug is blamed for a **~75 ms boot-time penalty**: *"The
  75ms difference is attributed to Cloud Hypervisor's support for
  additional features (CPU/memory hotplugging, vhost-user devices…)."*

This is not a contradiction of Overdrive's rationale — it is the boundary
condition on it. **Hotplug only pays for itself on long-lived workloads.**
algiers ran short-lived fire-and-forget jobs, so it paid the boot cost and
got nothing. Overdrive has Service-kind workloads, a right-sizing
reconciler (#92), and persistent microVMs (#96–#100), so it collects.
DISCUSS should record the ~75 ms as an accepted, quantified cost — not
discover it later.

## Open questions DISCUSS must answer

1. **RESOLVED at intake — Cloud Hypervisor, on CPU hotplug + virtiofs.**
   See "The VMM premise" above. Residual work is wording, not decision:
   stop citing memory hotplug as a discriminator, and route the
   whitepaper:535 matrix correction through the architect.
2. **What exactly does "BYO artifact" mean in the TOML?** Kernel path,
   rootfs path, cmdline, vCPU/memory — and which of those are operator
   surface vs derived.

2. **RESOLVED at intake (I-5) — see "Decision I-5" below.** The TOML
   table is `[vm]`, `DriverType::MicroVm` is deleted, `DriverType::Vm`
   survives.
3. **Guest agent in slice 1 — yes or no?** The OCI research recommends
   *yes*, minimal (~200 lines, PID 1), specifically because Overdrive's
   restart/backoff model is driven by observed exit status, which an
   agentless VM cannot report. This directly determines whether
   `ExitEvent` can be honest for microVMs.
4. **How does a `vm` allocation reach `Running`?**
   **CORRECTED 2026-08-01 by the DISCUSS wave — the original wording here
   was wrong.** This intake claimed `ExecDriver` derives `Running` from a
   live child PID. It does not. The shim sets `AllocState::Running`
   purely on `driver.start(&spec).await` returning `Ok`
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:1200-1206`) —
   there is no liveness observation at that seam for *any* driver.

   The consequence is sharper than the original framing: a VM driver whose
   `start()` returns `Ok` as soon as `vm.boot` returns 2xx **inherits the
   reference implementation's exact lie for free, with no code written and
   no reviewer able to point at a wrong line.** The fix belongs in the
   driver, not the shim — `VmDriver::start` must not return `Ok` until the
   guest has actually signalled readiness. See `[G3]` / `[D2]` in
   `feature-delta.md`.
5. **Does the existing netns/veth/mTLS layer compose?** The action-shim's
   `provision_and_inject_netns` C3 seam is driver-agnostic and hands
   `spec.netns` / `host_veth` / `workload_addr` to the driver. A CH VM
   needs a tap device *inside* that netns and an in-guest NIC config
   mechanism. Confirm, do not assume.
6. **Rootfs format for slice 1.** The OCI research recommends `ext4` +
   `virtio-blk` and explicitly warns off virtiofs-**DAX** on CH (unsupported
   upstream; CVE-2026-24834 drove Kata back to `virtio-blk-pci` on CH in
   3.27.0).

   **CORRECTED 2026-08-02 — this question was scoped to the ROOTFS and must
   be read that way.** A prior version of this bullet ended *"the algiers
   precedent — virtiofsd — is the option the most experienced team retreated
   from on this exact VMM."* That fused two unrelated facts and is struck:
   Kata retreated from **`virtio-pmem`**, not from virtiofsd, and algiers
   never put its rootfs on virtiofs (`architecture.md:196` — block devices
   for `code.ext4` / `deps.ext4`; virtiofs only for the writable `/output`
   share). Answering this question therefore says **nothing** about volumes.
   **That omission is exactly how `I-6` came to be reversed** — see Decision
   I-6 above and `feature-delta.md` `[D5]` / `[D8]`.

## Prerequisite work this feature does NOT include

Named here so it is not mistaken for planned work.

> **CORRECTED 2026-08-02.** This section previously read *"None of these has a
> GitHub issue yet."* That is now false for the image factory: the DISCUSS
> `[B4]` deferral was approved and filed as GH
> [#259](https://github.com/overdrive-sh/overdrive/issues/259). The other two
> entries below remain deliberately unfiled — `DriverRegistry` is a
> DISCUSS/DESIGN call inside this feature rather than a deferral, and
> `DriverType::Unikernel` is a separate feature, not a forward pointer this
> one owes.

- OCI / Dockerfile → rootfs image factory (the whole of I-3's deferral) —
  **tracked as GH [#259](https://github.com/overdrive-sh/overdrive/issues/259)**.
- Content-addressed image store, registry pull, layer cache.
- `DriverRegistry` — ADR-0022 (accepted 2026-04-27) deferred it, pre-
  committing the migration for exactly this moment: *"Phase 2+ adds the
  second driver class (`MicroVm` per whitepaper §6) and the registry
  pattern earns its keep at that point."* Whether it lands **in** this
  feature or before it is a DISCUSS/DESIGN call, not a deferral — the
  composition root is a single hardcoded `Arc<dyn Driver>`
  (`crates/overdrive-control-plane/src/lib.rs:1422`) and a second driver
  cannot be reached without changing it.
- `DriverType::Unikernel`. The Unikraft research is explicit: the guest
  differences (no `fork()`, no in-guest process tree, build-time-fixed
  process model) are `Driver` *contract* differences, not a config flag.
  Separate driver, separate feature.

## Grounding scan — current state of the codebase

Cited so DISCUSS does not re-derive it.

**Ready, no work needed:**

- `DriverType::MicroVm` exists with `as_str` / `FromStr` round-trip
  (`crates/overdrive-core/src/traits/driver.rs:44`, `:58`, `:79`).
- The `Driver` trait's optional surface all has defaults — a new impl
  must provide only `r#type`, `start`, `stop`, `status`, `resize`.
- `SimDriver` is already `DriverType`-parametric and
  `SimDriver::new(DriverType::MicroVm)` is exercised today
  (`crates/overdrive-sim/tests/acceptance/sim_adapters_deterministic.rs:360`).
- netns/veth provisioning, mTLS intercept, workload addressing, cgroup
  slice bootstrap, probe runner, exit observer, restart/backoff — all
  driver-agnostic.
- ADR-0030 §6 already sanctioned per-driver-class spec types and names
  `microvm_image: Option<ContentHash>`; ADR-0031 already reserves the
  `[microvm]` TOML table.

**Must be built:**

1. Driver dispatch — composition root is one hardcoded
   `Arc::new(ExecDriver::new(...))` at
   `crates/overdrive-control-plane/src/lib.rs:1422`.
2. Spec surface — `DriverInput::MicroVm`, `WorkloadDriver::MicroVm`. Six
   irrefutable `let WorkloadDriver::Exec(..) =` destructures become
   `match`es (deliberate tripwires per ADR-0031:197). `ParseError::MissingExec`
   (`crates/overdrive-core/src/aggregate/workload_spec.rs:743`) becomes
   "exactly one driver table".
3. `AllocationSpec` driver divergence — today a flat `command` / `args`
   with no driver discriminator, so the shim cannot route
   (`crates/overdrive-core/src/traits/driver.rs:133`).
4. The `Vmm` port trait + `SimVmm` (I-2).
5. `classify_driver_failure` — its `DriverType` parameter is explicitly
   unused and exec-shaped
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:198`).

## Precedent warnings from the reference implementation's own docs

algiers documented its CH migration thoroughly (a 592-line research doc,
8 DISCUSS artifacts, a 496-line `architecture.md`, 3 ADRs) and then
shipped 7 of 10 roadmap steps. The gap between its docs and its code is
the most useful thing in the reference — each item below is a failure
mode this feature is structurally exposed to.

1. **The one defect peer review CAUGHT is the one that shipped.** Review
   iteration 2 added roadmap step `03-01` ("refactor `FirecrackerRunner`
   to backend-agnostic via factory") precisely to close the runner-
   integration gap. Step 03-01 was never executed. Result:
   `create_virtualizer()` has zero callers and **no production code path
   can construct a CH VM** — `OPENCAPSULE_VMM=cloud-hypervisor` changes
   nothing. This is CLAUDE.md § "Build vertical slices through production
   entry points" happening in the wild: a complete mechanism, wired to
   nothing. Overdrive's equivalent step is the composition root at
   `crates/overdrive-control-plane/src/lib.rs:1422`. **If that line is
   still `Arc::new(ExecDriver::new(...))` when this feature closes, the
   feature has failed the same way.**
2. **The P0 blocking spike was never run.** SP-01 ("does the Unikraft
   qemu-target kernel boot under CH at all?") was rated `HIGH (blocker)`
   and *"Blocks all CH work"*; 43 KB of implementation was written on top
   of it anyway. Overdrive's analogue: does the pinned 6.18 appliance
   kernel (ADR-0068) boot under CH with the chosen rootfs format, on the
   Lima dev kernel *and* the appliance kernel? Ground it before, not
   after.
3. **Exit-code reporting was flagged and never designed.** Their research
   filed it as "Gap 4: Cloud Hypervisor VM Exit Signaling"; nothing was
   built. Their `wait()` observes the **host `cloud-hypervisor` process**
   exit, not the guest workload's — a guest that boots, panics, and shuts
   down cleanly still exits the VMM `0`. They shipped an acceptance
   criterion (*"Job completion status is reported correctly
   (success/failure)"*) that nothing in the design could satisfy. This is
   intake open question 4, and it is the single most likely place for
   this feature to ship a lie: Overdrive's `ExitEvent` /
   `ExitKind::CleanExit | Crashed` contract, its restart/backoff
   reconciler, and `workload describe` all consume that signal.
4. **`Running` meant "the API returned 2xx".** No `vm.info` poll, no
   serial watcher, no readiness beacon — state flipped to `Running`
   immediately after `vm.boot` returned. Intake open question 4 again.
5. **The trait shape was frozen first, then type-safety was rejected to
   fit it.** ADR-0002 rejected a type-state builder *"because the
   Virtualizer trait uses `&mut self`"* — i.e. the constraint was the
   pre-existing trait, not the problem. Their own research had suggested
   extending the trait (`research:468`); the suggestion was silently
   dropped between research and design with no recorded rationale. The
   consequence is a stringly-typed escape hatch: `attach_drive("output",
   …)` is intercepted on the magic string `"output"` and means "spawn a
   virtiofsd sidecar and reconfigure shared memory" on one backend and
   "attach a block device" on the other. **This is exactly the shape
   caveat in I-2** — design the `Vmm` trait to the problem, and if the
   `Driver` trait needs to change to accommodate it, change it.
6. **Security posture drifted from the research with no record.**
   `architecture.md:406` claims *"VM isolation identical to
   Firecracker"*, contradicted by their own `research:430` (CH has **no
   jailer**; compensating systemd hardening was prescribed and never
   built). virtiofsd's sandbox mode also silently flipped from the
   researched `namespace` to the weaker `chroot` across every downstream
   doc. Overdrive inherits the no-jailer fact: the `cloud-hypervisor`
   process is just a host process and needs the same cgroup/seccomp
   treatment `ExecDriver` gives workloads.

   **ADDRESSED 2026-08-02 — see `feature-delta.md` `[D7]`.** The claim is
   now locked and quoted (*KVM + default-on seccomp + Landlock +
   cgroup/netns confinement; **no** jailer-equivalent chroot, **no** PID
   namespace*), six concrete hardening items are folded into the feature,
   and the jailer remainder is GH
   [#258](https://github.com/overdrive-sh/overdrive/issues/258).
   *"VM isolation identical to Firecracker"* is a named forbidden sentence
   under system constraint 6. Two facts this warning did not capture: CH's
   seccomp is **on by default** with per-thread filters, and CH offers
   **Landlock** (`--landlock`) — a better answer than chroot for the
   filesystem half, since it holds post-compromise and builds no jail tree.
7. **Magic version floors.** CH ≥ 48.0 and virtiofsd ≥ 1.10 are asserted
   in six documents with **no stated reason anywhere** — no API change, no
   feature landing, no distro-availability argument. The startup check
   that was supposed to enforce them was never built. If this feature
   declares a CH version floor, it must say what breaks below it.
8. **Networking was never designed.** Zero hits for `tap` / `nft` /
   `dhcp` / `bridge` across their entire CH corpus; `VmConfig` has no
   `net` key. A job under their CH backend would have had no NIC at all —
   which vacuously "satisfied" their egress allowlist. Overdrive's
   equivalent trap is intake open question 5 (#222): the existing
   netns/veth/nft-TPROXY layer assumes a host `struct sock`, and a microVM
   has none.

## Research inputs

Commissioned at intake, all three complete:

- `docs/research/platform/fly-io-microvm-implementation-research.md` —
  30 sources. Firecracker + LVM2 thin-pool block devices + Rust `init` on
  its own block device + `flyd` (durable FSM per operation). **Note: the
  public `superfly/firecracker` fork is a 2020 v0.24.6 artifact and is
  NOT what Fly runs.**
- `docs/research/platform/unikraft-microvm-and-dockerfile-reuse-research.md` —
  38 sources. Four distinct Dockerfile-reuse mechanisms, commonly
  conflated. **Cloud Hypervisor is not a supported Unikraft target.**
- `docs/research/platform/oci-image-to-microvm-rootfs-research.md` —
  41 sources. Every mature implementation splits the VM's own rootfs from
  the workload's OCI rootfs. **virtiofs DAX does not exist on Cloud
  Hypervisor.**

Pre-existing and directly relevant:

- `docs/research/platform/firecracker-vs-cloud-hypervisor.md` (2026-04-19)
- ADR-0022 (`AppState.driver`, registry deferred), ADR-0029
  (`overdrive-worker` is the home for driver impls), ADR-0030 §6
  (per-driver-class specs), ADR-0031 (tagged `WorkloadDriver` /
  `DriverInput`), ADR-0068 (pinned appliance kernel).
