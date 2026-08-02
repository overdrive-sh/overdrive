# Slice 01 — Walking skeleton: a `[vm]` job boots under Cloud Hypervisor and its real exit code reaches the operator

> DISCUSS brief (2026-08-01). Feature: `microvm-driver-cloud-hypervisor` (GH #42).
> Story: **US-VM-1**. Job: **J-OPS-003**. **The walking skeleton.** Gated by Slice 00
> PROMOTE.

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
- **Rootfs:** ext4 attached as `virtio-blk` (`[D5]` — the **rootfs** decision; it says
  nothing about virtiofs, which `[D8]` selects for volumes in Slice 04); per-launch
  `cp --reflink=auto` so the operator's artifact is never mutated. The copy is discarded at
  terminal **by design** — which is what makes restart honest, and also why a workload
  needs a Slice 04 volume for any output to survive.
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
      `/proc/<vmm-pid>/status`'s non-zero `Seccomp:` mode is retained as the runtime
      regression guard; note it is satisfied by CH's default alone, so it does not by itself
      prove this slice acted.
- [ ] **PID resolution** — every `/proc/<vmm-pid>/…` assertion resolves the hypervisor PID
      via the allocation's cgroup scope `cgroup.procs`. This makes item 5's cgroup placement
      a **prerequisite for verifying US-VM-7 in Slice 03**, not just an ordering.
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
- SHIPPED and reused unchanged: reconciler runtime, action shim, exit observer,
  restart/backoff, cgroup slice bootstrap, `workload describe`.
- ADR-0022 pre-committed the registry migration to "the second driver class" — this is it.
  ADR-0030 §6 pre-sanctioned per-driver-class spec types.
