# Slice 01 — Walking skeleton: a `[vm]` job boots under Cloud Hypervisor and its real exit code reaches the operator

> DISCUSS brief (2026-08-01). Feature: `microvm-driver-cloud-hypervisor` (GH #42).
> Story: **US-VM-1**. Job: **J-OPS-003**. **The walking skeleton.** Gated by Slice 00
> PROMOTE.
>
> **`superseded-by-DESIGN` (2026-08-11, GH #42).** Four statements below are
> corrected in place by the DESIGN wave; the governing text is named at each
> site, and **the ADR/brief section governs where this file and it disagree**.
> Summary of what changed for this slice — **C-1** the per-launch copy is the
> **`FICLONE` ioctl**, never `cp --reflink=auto` (ADR-0082 § D2 / brief § 102);
> **C-2** every `--disk` carries `image_type=raw`, rendered at one site
> (ADR-0082 § D2.1); **C-3** `memory.max` is `guest RAM + reserve`, never the
> declared figure alone (brief § *System Architecture* SD-4, ADR-0082 § D2);
> **C-5** the seccomp runtime guard reads `/proc/<pid>/task/*/status`, because
> `/proc/<pid>/status` reports `Seccomp: 0` on a **correctly** confined CH
> (spike P5) — the old AC **failed against correct behaviour**. **C-6**
> (`RLIMIT_FSIZE = max(rootfs, guest RAM)`) is encoded from this slice even
> though `shared=on` first ships in Slice 04.
>
> **Fold-in, 2026-08-11 (same DESIGN pass, additions rather than corrections)** —
> per user ruling, three prerequisites this feature needs *to work properly*
> land now rather than as deferrals: **(a)** `overdrive-init`'s two static musl
> build targets (ADR-0082 § D7 — see the Behavior section); **(b)** the D-3
> cgroup-OOM diagnosis gap, reduced form — a new `CgroupAccounting` port reads
> the scope's `memory.events` `oom_kill` counter at exit time, so a bad
> `reserve` lands `VmOutOfMemory`, not a bare `signal: 9` (ADR-0082 § D8,
> ADR-0083 § D5 row 13; see Behavior + Acceptance + the corrected Dependencies
> line). Item (c), coupling the `reserve_bytes` DELIVER measurement to (b)'s
> now-readable signal, is recorded in Slice 05.

## Goal (one line)

`overdrive deploy render.toml` with a `[vm]` + `[job]` spec boots a real Cloud Hypervisor
VM from a host kernel + ext4 rootfs, runs the operator's command **inside the guest**, and
`overdrive workload describe` reports the **guest's** exit code — driven end-to-end
through `overdrive serve` + `overdrive deploy`.

## Learning hypothesis

A `Vmm` port trait under the existing `Driver` trait is sufficient to make a VM workload
indistinguishable from a process workload at the operator surface, **without changing the
action shim** — because moving the honesty into `VmDriver::start`'s return value makes the
shim's existing `Ok → Running` mapping (`action_shim/mod.rs:1200-1206`) correct rather
than requiring a parallel readiness seam.

**Predicted:** the guest's exit code arrives at `workload describe` byte-exact, the
allocation never reports `Running` for a guest that did not start, and no shim change is
needed.

## Thinnest `serve` + `deploy` loop

`overdrive serve` (one node) + `overdrive deploy render.toml`, where the spec is:

```toml
[job]
name = "batch-render"

[vm]
kernel  = "/var/lib/overdrive/artifacts/vmlinux-6.18"
rootfs  = "/var/lib/overdrive/artifacts/render.ext4"
command = "/usr/bin/render"
args    = ["--frames", "120"]

[resources]
cpu_milli    = 1000
memory_bytes = 1073741824
```

Job kind only. **No networking, no probes, no mesh, no mTLS, no service ports, and no
volumes.** One VM, runs to completion, reports its exit code.

> **Volumes are deliberately below the skeleton line.** `[D8]` puts shared writable storage
> in **Slice 04**, with its own supervised `virtiofsd` process. A VM that boots and reports
> an honest exit code is already a complete end-to-end loop; adding a sidecar here would
> fatten the skeleton — the named anti-pattern — and this slice is over budget already.
> Consequence for this slice: `--memory shared=on` is **off** (`[D8b]` derives it from
> volume presence, and there are none), and no storage daemon is started.

## Behavior (DESIGN owns the API shape)

- **Spec:** `[vm]` table (`kernel`, `rootfs`, `command`, `args`). `[exec]` stops being
  unconditionally required — the parse contract becomes **exactly one driver table**.
  `[vm]` + `[service]` is out of scope here (Slice 02 / US-VM-6).
- **Intent:** `WorkloadDriver::Vm(..)` + `DriverInput::Vm(..)`. **rkyv schema-evolution
  event** — `WorkloadDriver` is embedded in the persisted `Job` aggregate. **`JobEnvelope`
  goes V1 → V2 via the full six-step single-commit procedure** (`.claude/rules/development.md`
  § "rkyv schema evolution" → "Version-bump procedure"). **This is user-ruled 2026-08-02 and
  is not a DESIGN judgement call** — mutating `JobV1` in place was considered and rejected
  because it requires **regenerating a golden fixture**, the one move
  `.claude/rules/testing.md` forbids outright (*"prior `FIXTURE_V_N` literals are never
  touched"*). The existing V1 fixture stays untouched and becomes the V1→V2 evidence, so the
  genuinely new work is one `From<JobV1> for JobV2` impl plus one new fixture. Eleven
  irrefutable `let WorkloadDriver::Exec(..) =` destructures become `match`es.
- **Port:** `Vmm` in `overdrive-core` — a `VmConfig` **value** plus a single
  `Vmm::create(&VmConfig)`, so "boot before configured" is *unrepresentable*, not
  runtime-validated. **Do not** reproduce the reference implementation's
  `configure → set_boot_source → attach_drive → start` state machine. Cloud Hypervisor is
  the **only** implementor in scope.
- **Adapters:** `CloudHypervisorVmm` (`adapter-host`) and `SimVmm` (`overdrive-sim`).
- **Driver:** `VmDriver: Driver` in `overdrive-worker` over `Arc<dyn Vmm>`; owns the
  allocation-shaped concerns `ExecDriver` already models (exit watcher, the
  Running-confirmed gate, cgroup placement of the hypervisor process).
- **Hypervisor placement (`[D7]` items 5–6) — inherent to `start`, not added to it.**
  `VmDriver::start` has `ExecDriver::start`'s four-step shape: create the workload cgroup
  scope → write resource limits → pre-open the netns FD → spawn with `setns(CLONE_NEWNET)`
  in a `pre_exec` hook. Two reasons this is *in* the skeleton rather than deferred:
  (a) without the scope, `cgroup_kill` has nothing to kill at stop (Slice 03); (b)
  `provision_and_inject_netns` gates on `mtls_worker.is_none()` — **not on driver type**
  (`action_shim/mod.rs:839`) — so on the mTLS-composed production boot a VM alloc is handed
  `spec.netns` and a matching teardown whether or not the driver enters it. Ignoring it
  leaves a provisioned-then-destroyed netns nothing ever entered. A Job-kind VM needs no tap
  inside it; an empty netns is *stronger* confinement, not a gap. Additionally: **the driver
  constructs the seccomp argument explicitly rather than relying on CH's default**, and it is
  never `false` or `log` — explicit construction is what gives the negative property a real
  mutation site, and this slice must not be free to turn seccomp off for a later slice to
  turn back on.
- **Dispatch:** `crates/overdrive-control-plane/src/lib.rs:1422` stops hardcoding a single
  `Arc::new(ExecDriver::new(...))`; the shim routes by the spec's driver table.
- **Guest agent:** static PID-1 `overdrive-init` — ready beacon, exec the command,
  forward stdio, report the real exit status over vsock. Not `kata-agent`.
  **Engineering constraint (M-5's toolchain half, folded in — DESIGN, 2026-08-11):**
  `overdrive-init` is a new `crate_class = "binary"` crate built **static** for
  **`{x86_64,aarch64}-unknown-linux-musl`** (ADR-0082 § D7 — already decided
  there; recorded here so a DISTILL/roadmap author sees it, since it appeared in
  zero slice files before this pass). Add both musl targets to the toolchain and
  CI in this slice's step that builds the crate. **There is no publish pipeline —
  not in this slice and not later.** BYO-artifact is intake **I-3**'s *slicing
  mechanism* (the `[vm]` spec points at a prebuilt kernel + rootfs already on the
  host), adopted so this driver ships without blocking on the image factory —
  *thinner-but-live beats complete-but-dead*. It is **not** an operator-facing
  product surface, so nothing obliges the platform to publish `overdrive-init` as
  a consumable artifact carrying a host↔guest protocol-compatibility contract.
  Getting `overdrive-init` into a rootfs is the **factory's** job, GH
  [#259](https://github.com/overdrive-sh/overdrive/issues/259), which now carries
  that requirement. *(User ruling 2026-08-11. GH #264 was filed on the opposite
  premise — that BYO rootfs is a product surface — and is closed as wrong-premised,
  rather than deferred.)* The crate and its two build targets are the whole of
  this slice's obligation.
- **Cgroup-OOM diagnosis (deferral D-3, reduced form — folded in, DESIGN
  2026-08-11; ADR-0082 § D8, ADR-0083 § D5 row 13):** the exit-watcher this
  slice builds reads the allocation's cgroup scope `memory.events` `oom_kill`
  counter, once, immediately after the watcher's `wait`/`recv()` resolves and
  before any teardown, via a new `CgroupAccounting` port (`overdrive-core`,
  beside `CgroupFs`; `RealCgroupAccounting` in `overdrive-host`,
  `SimCgroupAccounting` in `overdrive-sim`; composed and probed alongside
  `Vmm` under the same SD-5 composition gate — **not** unconditional like
  `VmHostState`). This is the **reduced** form of D-3 only: a post-mortem
  point read, not the live `memory.events` subscription D-3 names as its
  mechanism, and it covers the VM **mid-run** path only — an OOM during the
  30 s boot race still falls through to the existing boot-failure vocabulary
  (ADR-0083 § D5), unchanged by this fold-in. See ADR-0082 § D8 for the
  pinned trait, the `ExitEvent.oom: Option<OomFacts>` threading, and why
  neither `CgroupFs` (rejected already, ADR-0083 § A8) nor `VmHostState`
  (wrong cadence/crate boundary) is the right seam.
- **Rootfs:** ext4 attached as `virtio-blk` (`[D5]` — the **rootfs** decision; it says
  nothing about virtiofs, which `[D8]` selects for volumes in Slice 04); a per-launch clone
  so the operator's artifact is never mutated. The copy is discarded at
  terminal **by design** — which is what makes restart honest, and also why a workload
  needs a Slice 04 volume for any output to survive.
  **C-1 correction (DESIGN, ADR-0082 § D2 / brief § 102):** the clone is the **`FICLONE`
  ioctl issued directly**, **not** `cp --reflink=auto`. `auto` degrades to a full copy with
  **no error** on a filesystem that cannot reflink — measured 0.015 s / +0 MiB versus
  3.970 s / +4096 MiB, ~260× (P4) — and the ioctl has no `auto` path to degrade and no
  coreutils-version dependency. The clone lands on the **rootfs master's own filesystem**
  (reflink is intra-filesystem; staging into `/run` fails `Invalid cross-device link`), and
  its **filename carries the allocation id**, which is what makes a reboot-orphaned clone
  attributable (SD-1 / SD-2).
- **Disk argument (C-2, new here — no slice previously mentioned it):** every `--disk`
  carries **`image_type=raw` explicitly**, rendered at the single site
  `DiskAttachment::to_disk_arg` (ADR-0082 § D2.1). CH v53's auto-detect *"disables sector-0
  writes"*, and our images are bare filesystems where sector 0 **is** the filesystem — the
  guest faults, `panic=1` reboots it, and the failure surfaces two layers from its cause
  (P10/P11). There is no `image_type` field to forget.
- **Resource limits (C-3):** the allocation's `memory.max` is
  **`resources.memory_bytes + reserve(resources.memory_bytes)`**, not the declared figure
  alone (SD-4; `MemoryPlan::derive` is the only constructor, ADR-0082 § D2). The cgroup
  charges the hypervisor's whole RSS **plus** host page tables RSS cannot see, so setting
  both from one number is a cgroup OOM **by construction**, surfacing as
  `signal: 9` with no mention of memory. `reserve` is measured in DELIVER via
  `memory.current` / `memory.stat` — **not** RSS, and **not** guessed.
  **This is no longer undiagnosable when the guess is wrong** (deferral D-3, reduced
  form, folded in — ADR-0082 § D8): the exit-watcher this slice builds reads the
  scope's `memory.events` `oom_kill` counter at exit time, so a `reserve` that was too
  low lands `VmOutOfMemory`, not a bare `signal: 9`. See the Behavior section's
  "Cgroup-OOM diagnosis" bullet.
  **`RLIMIT_FSIZE` is `max(rootfs image, guest RAM)` from this slice (C-6)**, before
  Slice 04 turns `--memory shared=on` on, because `shared=on` backs guest RAM with a memfd
  and a memfd is a *file* for `RLIMIT_FSIZE`.
- **Single cut, same PR:** delete `DriverType::MicroVm`, regenerate OpenAPI, amend the
  now-false `traits/driver.rs:26-29` "wire form never changes" docstring.

## Carpaccio taste tests

- **Closes a real loop through production?** **Yes — and this is the feature's pass/fail
  bar.** `lib.rs:1422` changes; a real `serve` + `deploy` boots the VM. No acceptance test
  installs, binds, programs, or supplies anything `run_server` does not supply itself.
  Intake precedent warning #1 is exactly this failing.
- **Thinnest?** Job-kind only, one VM, no networking, no probes. The three candidate
  sub-splits each produce a *dead* increment rather than a *thin* one — see feature-delta
  § Scope assessment. **Deliberately the largest slice; its unknowns are absorbed by
  Slice 00.**
- **No `#[test]`-only composition?** Driven through `run_server` / the action shim /
  `deploy`, never a hand-assembled harness.
- **Ships a lie?** No — `[D2]` and `[D3]` are in this slice, not deferred. A slice whose
  deliverable is a plausible-looking lie is not a thinner slice.

## Acceptance (= US-VM-1 ACs)

- [ ] Driven end-to-end through `overdrive serve` + `overdrive deploy`; no test supplies a
      production effect.
- [ ] `lib.rs:1422` no longer hardcodes one `ExecDriver`; the shim routes by driver table.
- [ ] `[vm]` without `[exec]` parses; **both** or **neither** is rejected naming both tables.
- [ ] The exit code in `workload describe` is the **guest's** — Tier-3 case with guest exit
      **7** while the host `cloud-hypervisor` process exits **0**.
- [ ] `Running` only after the ready beacon; a rootfs with no working init reaches `Failed`
      **without** passing through `Running`.
- [ ] **`[D7]` item 5** — for a running VM alloc, `/proc/<vmm-pid>/cgroup` resolves to that
      allocation's workload scope and `/proc/<vmm-pid>/ns/net` is the inode of
      `/var/run/netns/<spec.netns>`, not the host netns. Read from `/proc` for the live
      hypervisor — not asserted against the placement call. **The Tier-3 case MUST run
      against an mTLS-composed `serve`** (production composition, `dataplane_override`
      unset): on an uncomposed boot `spec.netns` is never supplied (`[G6]`), and a
      conditionally-worded assertion would pass with **zero placement code written** — the
      GH #248 / ADR-0074 trap.
- [ ] **`[D7]` item 6** — the driver **constructs the seccomp argument explicitly** rather
      than relying on CH's default, so the negative property has a real mutation site; a
      mutation flipping it to `false` **or** `log` must be killed **by an assertion over the
      constructed argument**, not by the `/proc` read — CH's `log` mode still installs a
      filter, so `Seccomp:` stays non-zero and `log` would survive a `/proc`-only check.
      **C-5 correction (DESIGN, 2026-08-11; brief § 106):** the runtime regression guard
      reads **`/proc/<vmm-pid>/task/*/status`**, never `/proc/<vmm-pid>/status`. Spike P5
      measured `Seccomp: 0` on the thread-group leader of a **correctly** confined CH — the
      filters sit on the `vmm` / `http-server` / `vcpu0` threads — so the previous wording
      *"`/proc/<vmm-pid>/status`'s non-zero `Seccomp:` mode"* is **an AC that fails against
      correct behaviour** and must not be implemented. The per-thread read is retained as
      the runtime guard; note it is satisfied by CH's default alone, so it does not by
      itself prove this slice acted — which is why the argv assertion above is the binding
      half.
- [ ] **PID resolution** — every `/proc/<vmm-pid>/…` assertion resolves the hypervisor PID
      via the allocation's cgroup scope `cgroup.procs`. This makes item 5's cgroup placement
      a **prerequisite for verifying US-VM-7 in Slice 03**, not just an ordering.
- [ ] **C-1 / C-2 (added by DESIGN)** — the per-launch clone is produced by a **`FICLONE`
      ioctl** on the rootfs master's own filesystem (a mutation replacing it with a plain
      copy is killed), and **every** `--disk` argument the driver constructs contains
      `image_type=raw` (asserted over the constructed argument at the single rendering site,
      so it is a mutation target).
- [ ] **C-3 (added by DESIGN)** — the allocation's `memory.max` is **strictly greater** than
      the guest's RAM by the reserve; a VM booted at its declared `memory_bytes` is **not**
      cgroup-OOM-killed on reaching residency. `guest_bytes == cgroup_max_bytes` has no
      constructor.
- [ ] **Cgroup-OOM diagnosis (D-3 reduced form, added by DESIGN — ADR-0082 § D8,
      ADR-0083 § D5 row 13)** — a VM whose `memory.max` is deliberately set below its
      guest RAM (a `MemoryPlan` constructed with a `reserve` too small to hold residency)
      is classified `Failed / TransitionReason::VmOutOfMemory { limit_bytes,
      oom_kill_count }`, **not** `Failed / WorkloadCrashedImmediately { signal: 9 }` —
      Tier-3 case with `memory.max` set below the guest's declared `memory_bytes` on
      purpose. The scope's `memory.events` `oom_kill` counter is read once, immediately
      after the exit-watcher's `wait`/`recv()` resolves and before any teardown, via the
      new `CgroupAccounting` port. Restart/backoff is **unaffected** — `VmOutOfMemory`'s
      disposition is `StoppedBy::Process` (an ordinary crash), the **same** budget
      treatment as any other `Crashed` ending, **not** `PlatformReclaimed` (DD-1's third
      ending class is a different axis entirely — the platform losing supervision, not a
      supervised VM's cause of death). Scope boundary, stated so it is not mistaken for a
      gap in this AC: an OOM during the 30 s boot race (before the beacon wins D3's
      three-way `select!`) is unaffected by this AC and still falls through to the
      existing boot-failure vocabulary.
- [ ] `DriverType::MicroVm` deleted, OpenAPI regenerated, stale docstring amended — same PR.
- [ ] **`JobEnvelope` V1 → V2** lands as **one commit** via the six-step procedure, with a
      new golden-bytes fixture pinning V1 and a `From<JobV1> for JobV2` impl; **existing
      fixtures are never touched.** Not conditional — user-ruled.
- [ ] `SimVmm` exists and the VM path is reachable from Tier-1 DST.

## Dependencies

- **Slice 00 PROMOTE** (kernel boots under CH; vsock carries the beacon **including with the
  hypervisor placed in a netns — P2**; both kernels agree).
- **Size is unchanged at 5–8 d by the `[D7]` fold.** Items 5–6 add acceptance criteria to
  work already inside this slice; they add no mechanism. The additive confinement items
  (Landlock, uid/gid drop, rlimits) are **US-VM-7 in Slice 03**, and the mount namespace is
  **not in this feature** (GH #258) — see feature-delta `[D7]`.
  **RE-SIZED 2026-08-11 — the D-3 fold-in is NOT covered by the `[D7]` claim above.** The
  prior text on this line named the gap and explicitly declined to price it (*"sizing this
  honestly is DISTILL's job, not DESIGN's estimate to silently absorb"*). Closed out here,
  same pass, per user instruction not to leave it open. Costed the same way `[D8f]` costs
  Slice 04 — an itemized table, not a number pulled from the air:

  | Concern | Reused from | New | Est. |
  |---|---|---|---|
  | `CgroupAccounting` port + error types + full contract docstring (pre-/postconditions, edge cases, observable invariants — `.claude/rules/development.md` § "Trait definitions specify behavior") | `CgroupFs`'s trait shape | Yes | 0.5 d |
  | `RealCgroupAccounting` adapter (`memory.events` read + parse) + `probe()` with 3 fault-injection scenarios | mirrors `CgroupFs::probe`'s shape (ADR-0082 § D8) | Yes | 1 d |
  | `SimCgroupAccounting` adapter (in-memory `BTreeMap` + injectable per-path error schedule) | mirrors `SimCgroupFs` | Yes | 0.5 d |
  | Composition-root wiring — `VmDriver::new` gains a required param; SD-5's gate extended to probe `CgroupAccounting` alongside `Vmm` | SD-5's existing gate shape | Yes | 0.5 d |
  | Adapter equivalence test (Real vs Sim), mandatory per the trait-contract rule cited above | pattern from other port-equivalence tests in this codebase | Yes | 0.5 d |
  | `ExitEvent.oom: Option<OomFacts>` field + the exit-watcher's read-before-teardown call | the mid-run exit watcher this slice already builds | Yes | 0.5 d |
  | `exit_observer` precedence check + `TransitionReason::VmOutOfMemory` Cause variant + totality re-check | `[D3]`'s existing `Crashed → WorkloadCrashedImmediately` mapping | Yes | 1 d |
  | New Tier-3 AC — deliberately undersized `memory.max`, real cgroup OOM, confirm `VmOutOfMemory` lands, not `signal: 9` | this slice's own boot Tier-3 harness | Yes | 1 d |
  | `overdrive-init`'s two static musl build targets (toolchain + CI matrix) | — | Yes | 0.5 d |
  | **Subtotal — D-3 fold-in + M-5 toolchain** | | | **6 d** |

  **New band: 11–14 d** (the `[D7]`-fold band of 5–8 d, plus the 6 d subtotal above — none
  of it was in either prior number). US-VM-1's musl-toolchain share is 0.5 d of the 6; the
  load-bearing cost is the `CgroupAccounting` port + its two adapters + probe + equivalence
  test (3 d) and the new classification path they feed (2 d: the exit-observer branch/
  variant plus the Tier-3 AC that proves it).

  **Guardrail 3 fires (feature-delta § Scope assessment — wave-unadjustable, pre-committed
  at DISCUSS amendment 2's review fixes, 2026-08-02).** Slice 01's stated upper band was
  **8 d**; more than 50% over that is any upper bound past **12 d**. The re-sized upper
  bound is **14 d — 75% over the stated band.** This number is not chosen to duck the
  threshold and is not this wave's to resolve by picking a smaller one — see feature-delta's
  Changelog (2026-08-11) and § Scope assessment for the full trigger record and the
  total-effort consequence. Disposition is the user's call.
- SHIPPED and reused: reconciler runtime, action shim, restart/backoff, cgroup slice
  bootstrap, `workload describe`. **Correction (DESIGN, 2026-08-11) — exit observer is
  NOT unchanged.** `overdrive-control-plane`'s `worker::exit_observer::handle_exit_event`
  gains one additive precedence check ahead of its existing `Crashed →
  WorkloadCrashedImmediately` mapping (ADR-0082 § D8): a `DriverType`-agnostic check on
  `ExitEvent.oom` (a new `Option<OomFacts>` field `ExecDriver` never populates), so every
  Exec crash falls through unchanged and the "reused" claim holds for that half. The
  prior text asserted this file was untouched by the whole feature-delta; that was true
  before deferral D-3 was folded in and is corrected here rather than left to
  contradict the code.
- ADR-0022 pre-committed the registry migration to "the second driver class" — this is it.
  ADR-0030 §6 pre-sanctioned per-driver-class spec types.
