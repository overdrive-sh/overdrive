# Feature-Delta — microvm-driver-cloud-hypervisor (DISCUSS · DRAFT)

> **Status: DISCUSS authored 2026-08-01; amended 2026-08-02 (×2)** (B1–B4 ruled on, `[D7]`
> isolation claim locked, confinement folded; then **intake `I-6` recovered — storage
> splits by role, and volumes became Slice 04** — see § Changelog).
> The single authoritative DISCUSS
> narrative for this feature — the compact `feature-delta.md` form mandated by the
> `nw-discuss` Outputs contract; the legacy split `discuss/*.md` files
> (user-stories, story-map, dor-validation, outcome-kpis, wave-decisions) are
> intentionally **not** produced, their content lives here. Lean density, Tier-1
> `[REF]` sections. Produced by Luna (nw-product-owner) for GH
> [#42](https://github.com/overdrive-sh/overdrive/issues/42). Slice briefs under
> `slices/`.
>
> **Governing input:** `intake.md` (**six** locked decisions I-1..I-6, the verified
> VMM premise, eight precedent warnings). DISCUSS does not reverse those; it
> answers the six open questions as `[D1]`–`[D6]`, locks the isolation claim as
> `[D7]`, records the storage-by-role rule as `[D8]`, and sizes the work.
>
> **`I-6` is a recovered input, not a new idea.** The user's opening message named two
> reference-implementation decisions — *"speak with CH over unix socket and use virtiofsd
> for storage."* The first became `I-2`; **the second was dropped by the intake author and
> never recorded.** Research then recommended `ext4` + `virtio-blk`, the first DISCUSS pass
> scoped that, and the reversal was never surfaced. The recovery is `[D8]`. The
> load-bearing lesson: **the apparent research-vs-reference conflict was an artifact of
> DISCUSS scoping the rootfs and never scoping volumes at all** — virtiofs fell out by
> omission, not by decision.

## Reading checklist

- ✓ `docs/feature/microvm-driver-cloud-hypervisor/intake.md` — **I-1..I-6** (six locked decisions; `I-6` was recovered 2026-08-02 after being dropped), the VMM premise, the 8 precedent warnings from the reference implementation
- ✓ `docs/research/platform/oci-image-to-microvm-rootfs-research.md` — §4.1 (virtio-blk is CH's documented boot path), §4.4 (virtiofs DAX does not exist on CH), §4.6 (CVE-2026-24834 → Kata retreated to `virtio-blk-pci` on CH — the CVE concerns **`virtio-pmem` + DAX**, a different mechanism from virtiofs; see `[D8]`), §8.3 (the agentless cost analysis), § Recommendation
- ✓ **The reference implementation's own storage split** — `architecture.md:196`: *"Other drives (code.ext4, deps.ext4) continue as block devices in the `disks` VmConfig section"*; only `attach_drive("output", …)` was routed to virtiofs. **Block for the rootfs, virtiofs for the writable share — the research and the reference agree** (`[D8]`)
- ✓ `docs/research/platform/fly-io-microvm-implementation-research.md` — the Rust `init`-on-its-own-device pattern
- ✓ `docs/product/jobs.yaml` — **J-OPS-003** (the job this feature extends), J-OPS-004 (the honesty sibling), the udp-sendmsg4 elevation precedent (2026-06-05)
- ✓ `docs/product/personas/ana-platform-engineer.yaml` — Ana Moreno, who treats `Running` as a promise
- ✓ `docs/product/vision.md` — design principle 4 ("all workload types are first class")
- ✓ `CLAUDE.md` § "Build vertical slices through production entry points" — **the bar this feature is judged against**
- ✓ `.claude/rules/development.md` § "rkyv schema evolution" — the 6-step version-bump procedure (see `[G4]`, a cost the intake did not surface)
- ✓ `.claude/rules/spike.md` — governs Slice 00
- ✓ **Confinement evidence behind `[D7]`** — Firecracker `docs/jailer.md` (what a jailer
  actually does: `pivot_root` · mount ns · optional PID/net ns · uid/gid drop · cgroup ·
  `setrlimit` · `mknod`; seccomp applied by Firecracker itself, **not** the jailer) ·
  Cloud Hypervisor issue **#5170** (a jailer was proposed Feb 2023 and closed without one
  shipping) · CH `docs/seccomp.md` (**on by default**, per-thread filters for vCPU /
  virtio-device / HTTP-API / main VMM threads) · CH `docs/landlock.md` (opt-in
  `--landlock`; *"the process cannot access any resources outside of the ruleset during
  its lifetime, even if it were compromised"*; xattr access remains permitted)

---

## `[REF]` Persona

**Ana Moreno — Overdrive platform engineer** (`docs/product/personas/ana-platform-engineer.yaml`),
through her **lifecycle/ops** lens. She reasons in *intent vs actual state* and treats
`overdrive workload describe` as a **promise**, not a progress bar. Her standing
frustration: *a workload reported `Running` she cannot trust, and a diagnosis that
requires reading source instead of reading the CLI.*

That frustration is the whole feature. A VM is the first workload class Overdrive
**cannot directly observe** — the host sees a `cloud-hypervisor` process, not the
guest's workload. Every lifecycle signal Ana already relies on (`Running`,
`Terminated{Completed{exit_code}}`, `Failed{exit_code}`, restart/backoff) has to be
re-earned across a hardware boundary, or it becomes a lie she cannot detect.

> Per D3 (lightweight UX research): happy path + the failure paths, no elaborate
> emotional arc. The operator journey is the already-trodden `overdrive deploy <spec>`
> path; only the `[vm]` table and the failure vocabulary are new.

---

## `[REF]` JTBD — one-liner + job-tracing decision

**One-liner:** *When the workload I need to run cannot be a host process — it needs its
own kernel, its own OS, or hardware-level isolation — I want to declare it in the same
TOML and deploy it with the same verb, and have the platform tell me the truth about
whether it booted, whether it is running, and how it ended — so that adding a workload
class does not cost me the lifecycle honesty I already trust for processes.*

### Job-tracing decision: EXTEND `J-OPS-003` (do NOT mint a new job)

**Decision: EXTEND J-OPS-003** — *"Run my actual workload on the walking-skeleton
control plane and trust the platform to converge to the declared replica count"*, whose
outcome clause already reads *"…and `workload describe` honestly reflects whether each
allocation is Pending (with a reason), Running, Draining, Terminated, or Failed."*

**Rationale (one line):** running a VM-class workload is the **same convergence job
along the workload-class dimension** — same progress (converge declared intent into
running workloads, report state honestly), same actor-circumstance (Ana running her
actual workload on the single-node control plane), same failure-mode class (intent not
converged, or state dishonestly reported).

This follows the **udp-sendmsg4 elevation precedent** (jobs.yaml changelog 2026-06-05:
*"one job spans the protocol/syscall-idiom dimension; a per-idiom job would fragment
J-OPS-004"*). A per-driver-class job would fragment J-OPS-003 identically — and
`vision.md` design principle 4 (*"All workload types are first class… one control
plane, one identity model"*) says explicitly that workload class is a **dimension of
one job**, not a new one.

> **Why not mint a new job?** Considered on two framings, both rejected.
> **(a) "Run a VM workload"** would carry J-OPS-003's exact outcome statement — the
> J-SEC-002 mint bar ("genuinely distinct progress + failure mode, independently
> satisfiable and independently failable") is not met: if the VM does not converge,
> J-OPS-003 has already failed.
> **(b) "Run an untrusted / OS-divergent workload with hardware-level isolation"** —
> this *would* be a genuinely distinct progress (an isolation guarantee, not a
> convergence guarantee) with a distinct failure mode (escape). This feature delivers a
> **bounded, named isolation posture** (`[D7]`) — KVM + default-on seccomp + Landlock +
> cgroup/netns confinement — but **not an isolation guarantee**: there is no
> jailer-equivalent chroot and no PID namespace, and the written guest→VMM→host threat
> model is GH [#258](https://github.com/overdrive-sh/overdrive/issues/258). A job whose
> progress is "escape does not happen" cannot be minted against a posture whose own
> threat model is unwritten. It is a genuine future mint **when #258 closes.**

**Relates to J-OPS-004.** The Running-lie this feature must not ship is exactly
J-OPS-004's shape one class over: *"kernel-accepted exec is NOT operator-meaningful
liveness"* becomes *"VMM-accepted boot is NOT operator-meaningful liveness."* J-OPS-004
is Service-kind + probes; slice 01–04 here are Job-kind, so this is a `relates_to`
link, not an elevation target.

---

## `[REF]` Grounding — what is true today (do NOT re-derive)

Verified against the working tree 2026-08-01. Five findings; **`[G3]` and `[G4]` correct
or extend the intake.**

### `[G1]` The composition root is one hardcoded driver — this is the pass/fail bar

`crates/overdrive-control-plane/src/lib.rs:1401-1428`, `compose_production_driver`:

```rust
let driver: Arc<dyn Driver> = Arc::new(                       // :1422
    overdrive_worker::ExecDriver::new(cgroup_root, clock, fs)  // :1423
        .with_probe_runner(Arc::clone(&probe_runner)),
);
```

`AppState.driver: Arc<dyn Driver>` (`lib.rs:198`) is **one** driver. There is no
registry, no factory, no selector. The action shim calls `driver.start(&spec)`
unconditionally. **A second driver class cannot be reached without changing line 1422
and giving the shim a way to route.** Per intake precedent warning #1 — the reference
implementation's `create_virtualizer()` had zero callers and shipped anyway — **if line
1422 is unchanged when this feature closes, the feature has failed the same way.**

### `[G2]` `DriverType::Vm` already exists; only `MicroVm` is deleted

`crates/overdrive-core/src/traits/driver.rs:45` already carries `Vm` with `as_str →
"vm"` (`:59`) and `FromStr("vm")` (`:79`). I-5 is therefore a **deletion-only** change
to the enum: drop `MicroVm` (`:43`, `:58`, `:78`), amend the now-false *"existing
variants never change their wire form"* docstring (`:26-29`), regenerate OpenAPI
(`DriverType` derives `ToSchema` via `TransitionSource`). Two exhaustive matches lose an
arm — compiler-enforced.

### `[G3]` The `Running` lie is **structural and lives in the shim, not the driver**

The intake says *"`ExecDriver` gets this from a live child PID."* It does not.
`crates/overdrive-control-plane/src/action_shim/mod.rs:1200-1206`:

```rust
) = match driver.start(&spec).await {              // :1200
    Ok(handle) => (
        Some(handle),
        AllocState::Running,                       // :1203
        Some(TransitionReason::Started),           // :1204
```

**The gate is literally "`driver.start` returned `Ok`."** `ExecDriver::start` returns
`Ok` immediately after `cmd.spawn()` (`overdrive-worker/src/driver.rs:471`, `:581`) —
which is precisely why RCA-A and J-OPS-004 exist.

**This is load-bearing for the feature.** A `VmDriver` that returns `Ok` when the VMM
accepts the boot request inherits the reference implementation's lie **for free,
structurally, with no new code written.** It is the default outcome, not a mistake
someone has to make. See `[D2]` for the answer, which needs no shim change.

### `[G4]` Adding a driver variant is an **rkyv schema-evolution event** (intake missed this)

The intake verified that `DriverType` never reaches a persisted row — correct, but that
is the wrong type. **`WorkloadDriver`** (`crates/overdrive-core/src/aggregate/mod.rs:162-167`)
is rkyv-persisted, embedded in the `Job` aggregate, with golden-bytes fixtures at
`crates/overdrive-core/tests/schema_evolution/workload_intent.rs:53,73,96`.

Per `.claude/rules/development.md` § "rkyv schema evolution": *"Aggregate envelopes wrap
the outer type only… Embedded type changes bump the outer envelope version."* Adding
`WorkloadDriver::Vm(Vm)` therefore lands the **6-step `Job` envelope version-bump
procedure in a single commit**, including a golden-bytes fixture pinning V1 that is
**never touched again**.

Appending the variant preserves existing rkyv discriminant tags, but a larger payload
changes the enum's archived size and shifts subsequent `Job` field offsets.

**RULED by the user 2026-08-02 — do the proper bump. This is no longer a DESIGN
question.** `JobEnvelope` **V1 → V2**, the full **six-step single-commit procedure**
(`.claude/rules/development.md` § "rkyv schema evolution" → "Version-bump procedure").

The cheaper alternative — mutate `JobV1` in place and regenerate its golden fixture — was
considered and **rejected**. It is affordable only because nobody is in production, and it
requires **regenerating a golden fixture**, which is the one move
`.claude/rules/testing.md` explicitly forbids: *"prior `FIXTURE_V_N` literals are **never**
touched."* Greenfield buys the right to skip *migrations*, not the right to erode the
schema-evolution signal.

**The real cost is therefore bounded and known**: the existing V1 fixture stays untouched
and is reused as the V1→V2 evidence, so what is actually new is one `From<JobV1> for JobV2`
impl plus one new fixture pinning V1's bytes. Budget it inside Slice 01; do not discover it
there.

### `[G5]` `AllocationSpec.command`/`args` are **reusable** — the divergence is smaller than it looks

`AllocationSpec` (`traits/driver.rs:132-234`) derives only `Debug, Clone, PartialEq, Eq`
— **no serde, no rkyv**; it is recomputed each reconcile tick, so changing it costs no
schema evolution. And under BYO-artifact the operator still names an entrypoint, so
`command`/`args` stay meaningful: the **guest agent execs them inside the guest**. The
divergence is therefore additive artifact fields, not a restructure of the exec-shaped
core. Eleven irrefutable `let WorkloadDriver::Exec(..) =` destructures (5 production, 6
test) become `match`es — deliberate tripwires per ADR-0031:197, compiler-enforced.

### `[G6]` Probes and netns are structurally driver-agnostic but mechanically host-coupled

- `provision_and_inject_netns` (`action_shim/mod.rs:830-868`) is gated on
  `mtls_worker.is_none() → return Ok(())` (`:839`). With mTLS uncomposed, a VM alloc
  gets `netns = None`, `workload_addr = None` — the existing behavior.
- `ExecDriver` enters the netns via `pre_exec` + `setns` (`driver.rs:452`). **A VM needs
  a tap device inside that netns instead** — a different mechanism at the same seam.
- The probe runner holds **no `Driver` and no `DriverType`** (`probe_runner/mod.rs:72-123`)
  — good. But TCP/HTTP probes default to `127.0.0.1` (`:417-422`) and the `Exec` mechanic
  joins the workload's **cgroup scope** (`:509`) and forks. **Neither reaches inside a
  guest.** Probes therefore gate Service-kind VMs, not Job-kind — see `[D6]`.
- `classify_driver_failure` (`action_shim/mod.rs:198-235`) confirms `_driver: DriverType`
  is **unused**; the prefix table is exec-shaped only (`spawn …`, `cgroup setup failed: …`).
  Every VM failure falls through to `DriverInternalError { detail }` (`:234`) — a raw
  string, not an operator diagnosis. See `[D3]`.

---

## `[REF]` Decisions — the six intake questions, plus two locked rules

`[D1]`–`[D6]` answer the intake's six open questions. `[D7]` locks the isolation claim.
`[D8]` records intake **`I-6`** — the storage-by-role rule the first intake pass dropped.

### `[D1]` What `[vm]` contains — two required paths, one entrypoint, resources reused

```toml
[job]
name = "batch-render"

[vm]
kernel  = "/var/lib/overdrive/artifacts/vmlinux-6.18"   # required
rootfs  = "/var/lib/overdrive/artifacts/render.ext4"    # required
command = "/usr/bin/render"                             # required — execed IN the guest
args    = ["--frames", "120"]                           # optional

[resources]
cpu_milli    = 2000            # NOT duplicated in [vm]
memory_bytes = 2147483648
```

| Field | Surface | Rationale |
|---|---|---|
| `kernel`, `rootfs` | **Operator** | The whole of BYO-artifact (I-3). Host paths; no registry, no image factory. |
| `command`, `args` | **Operator** | Mirrors `[exec]`; carried in the existing `AllocationSpec.command`/`args` (`[G5]`) and execed in-guest by the agent. |
| vCPU count | **Derived** from `resources.cpu_milli` | One source of truth. `[resources]` is driver-agnostic and is what the #92 right-sizing reconciler reads; duplicating it in `[vm]` would create the two-sources bug the CPU-hotplug story depends on not having. Derivation rule (round-up, floor 1) is DESIGN's. |
| memory size | **Derived** from `resources.memory_bytes` | Same. |
| **`cmdline`** | **NOT operator surface** | Deliberate. A wrong `root=` bricks the boot and the platform cannot tell Ana why — it looks identical to a corrupt rootfs. Material honesty says do not ship a footgun whose failure we cannot diagnose. The platform derives `console=ttyS0 root=/dev/vda rw init=<agent>`. Add it later if a real need appears; removing it later is a breaking change. |
| vsock CID, API socket path, per-launch rootfs copy, disk attach mode | **Platform** | Allocation-scoped, not operator concerns. |

**Parse contract:** `[exec]` stops being unconditionally required
(`workload_spec.rs:743-745`, `ParseError::MissingExec`) and becomes **"exactly one
driver table"** — `[exec]` xor `[vm]`. All three workload kinds (`ServiceSpec`,
`JobSpec`, `ScheduleSpec`) currently carry a non-optional `exec: ExecInput`; the change
is uniform across them.

### `[D2]` How a `vm` allocation honestly reaches `Running` — block `start` on a guest ready beacon

**Decision: `VmDriver::start` does not return `Ok` until the guest agent's ready beacon
arrives over vsock, or the boot deadline expires.**

This is the answer that needs **no shim change**. Given `[G3]` — the shim maps `Ok` →
`Running` — moving the honesty into `start`'s return makes the existing seam correct
rather than inventing a parallel one. `Running` then means *"the guest kernel booted and
the agent ran"*, which is **strictly stronger** than `ExecDriver`'s *"a process was
spawned."* Adding a workload class does not regress the lifecycle contract.

Rejected: **`Running` on a 2xx from `vm.boot`** — the reference implementation's
behaviour (intake precedent warning #4). It asserts nothing about the guest: a VM that
boots to a kernel panic, or whose rootfs has no working init, returns 2xx identically to
one that works. It is a lie that is *cheaper to write than the truth*, which is exactly
why it shipped there.

Boot-deadline expiry maps to `DriverError::StartRejected { reason }` → `Failed` with a
VM-shaped cause (`[D3]`), never a hung `Pending`.

### `[D3]` How workload exit is observed — three distinct situations, none collapsed

**This is the single most likely place for this feature to ship a lie** (intake
precedent warning #3: the reference implementation shipped an acceptance criterion
nothing in its design could satisfy). The consumers are real and already tested:
`exit_observer::classify` (`worker/exit_observer.rs:610-636`) and the restart/backoff
branch in `workload_lifecycle.rs:673-743`.

| Situation | Honest classification | The lie to refuse |
|---|---|---|
| Agent reported the workload's exit status | `CleanExit` / `Crashed { exit_code, signal }` from the **guest's** status | — |
| VMM process exited with **no** agent report (kernel panic, VMM crash, host OOM-kill) | `Crashed` with a cause naming *the guest died un-reported* | **Mapping `cloud-hypervisor` exit 0 → `ExitKind::CleanExit`.** A guest that boots, panics, and shuts down cleanly exits the VMM `0`. |
| Operator stop | `intentional_stop: true` | Classifying an operator stop as a crash and burning restart budget |

**The named anti-pattern:** deriving `ExitKind` from the **host** `cloud-hypervisor`
process's exit status. That is the reference implementation's `wait()`, and it is
undetectable from the outside — every VM would report success. The AC that kills it is a
Tier-3 case where the guest exits **7** and `workload describe` must show **7**.

**The rule generalises to every supervised sidecar, and Slice 04 is its second
application.** `[D8]` adds a `virtiofsd` process per volume-carrying VM. The reference
implementation got this wrong in **both** directions at once: its `wait()` treated *any*
`virtiofsd` exit — **including a clean one** — as `VmmError::Crash("virtiofsd crashed
unexpectedly")` and force-killed the VM; and nothing made a `virtiofsd` that died
*mid-run* reach `ExitKind` at all. So the stated rule is: **a supervised sidecar's death
is classified by whether the workload itself reported an outcome, never by the sidecar's
own exit status** — the identical shape as the VMM row above. See system constraint 9 and
**US-VM-9**.

### `[D4]` Guest agent in slice 1 — **YES**, the ~200-line version

**Decision: yes.** Not cargo-culted — `[D2]` and `[D3]` both require an in-guest signal,
and an agentless VM cannot produce one. Agentless would **silently degrade a
control-plane behaviour that already exists and is already tested** (restart/backoff
consumes observed exit status). That is a worse trade than ~200 lines of Rust.

Scope: a **static PID-1 `overdrive-init`** that (a) sends a ready beacon, (b) execs the
spec's `command`/`args` in the guest, (c) forwards stdio, (d) writes the real exit status
to **vsock**. Not `kata-agent` — that implements a full container runtime API Overdrive
does not need.

> **`[D4]` AMENDED 2026-08-02 with `[D8]` — the agent gains one duty, recorded explicitly
> rather than inferred.**
>
> **(e) mounts each declared volume at its `target` before exec'ing the command, and
> refuses to exec if a required mount fails.**
>
> This is stated as an amendment because it **expands a locked decision**, and an
> unrecorded expansion is the exact failure this whole amendment exists to correct. It is
> not optional: a secondary virtiofs share is **not** auto-mounted by the guest kernel —
> some in-guest process must issue `mount -t virtiofs <tag> <target>`, and PID 1 is the
> only process Overdrive controls. `[D8]`'s first draft wrote *"and mounts the share in the
> guest at `target`"* in the passive voice with **no subject**, which is how a locked
> decision nearly got expanded silently.
>
> **Why the refuse-to-exec half is load-bearing, not defensive.** Without it the feature
> ships a lie of its own north-star class: if the mount silently fails, the guest command
> writes to `target` inside the **per-launch rootfs copy**, that copy is discarded at
> terminal *by design* (`[D5]`), the command exits 0, the agent honestly reports 0, and
> `workload describe` shows `Terminated / Completed{exit_code: 0}` **over an empty host
> directory**. Every signal is individually truthful and the composite is false. A failed
> required mount must reach `Failed` with its own named reason (US-VM-8), never a completed
> terminal state. vsock is confirmed universal across Kata, firecracker-containerd and
Lambda (research §8.1), CH supports it natively and hot-pluggably, and it works
**independently of guest networking** — which matters, because the agent must not depend
on the thing it may be helping configure.

**BYO-artifact contract:** the operator's rootfs must contain `overdrive-init` at a
well-known path; Overdrive ships the binary as a workspace build artifact. This keeps I-3
intact (no image factory) while getting the honest signal.
**Extended by `[D8]`:** a spec declaring a volume additionally requires that the operator's
**kernel** provides virtiofs support (`CONFIG_VIRTIO_FS`) and that the mount point can be
created in the guest. The pinned 6.18 appliance kernel does; an arbitrary BYO kernel may
not. **This is an operator-visible precondition of declaring a volume, so it fails as a
named reason and not as a mystery** — same US-VM-8 variant as any other failed required
mount. Stated here because BYO-artifact means the platform does not build the guest and
therefore cannot assume its capabilities. The research's structural
finding — *every mature implementation splits the VM's own rootfs from the workload's*
— points at Overdrive eventually shipping its own initramfs; that is image machinery and
is **out of scope**, but the agent contract is deliberately shaped so that move is
additive. GH [#100](https://github.com/overdrive-sh/overdrive/issues/100) is the full
`overdrive-guest-agent` (ttRPC, SPIFFE) this grows into.

### `[D5]` **Rootfs** format — `ext4` + `virtio-blk`, per-launch reflink copy

> **Scope: this decision governs the ROOTFS only.** Volumes / shared writable storage are
> `[D8]` and use **virtiofs**. `[D5]` says nothing about virtiofs as a mechanism — an
> earlier draft of this document wrote *"the driver uses ext4 + virtio-blk, not virtiofs"*,
> which reads as a rejection that was never decided. The rule is **by role**, not by
> preference.

**Decision: `ext4`, attached as `virtio-blk`.** Three pieces of evidence, each scoped to
what it actually shows:

- CH's own `device_model.md`: virtio-blk *"is usually used to boot the operating system
  running in the VM."* It is the documented boot path.
- **virtiofs DAX does not exist on CH** (`fs.md`: *"Given the DAX feature is not stable
  yet from a daemon standpoint, it is not available in Cloud Hypervisor"*; also
  unimplemented in the Rust virtiofsd). DAX is the mechanism that would buy cross-VM
  page-cache sharing for a virtiofs rootfs. Without it, a virtiofs **rootfs** costs a
  supervised per-VM daemon, forces `--memory shared=on`, and puts FUSE in the boot-read
  hot path (measured 0.78× native by nydus) while delivering none of that headline
  benefit. **This argues against virtiofs for the ROOTFS role specifically** — it is not
  an argument about virtiofs for a writable share, where there is no page-cache-sharing
  claim to lose.
- **CVE-2026-24834** (2026-02-19, CVSS 9.3) argues against a **pmem/DAX rootfs**, and
  that is all it argues. Kata DAX-mapped a read-only guest image over **`virtio-pmem`** on
  Cloud Hypervisor, took a container-to-guest escape, and the 3.27.0 remediation *"changes
  the VM rootfs driver from `virtio-pmem` to `virtio-blk-pci` for Cloud Hypervisor
  configurations."*
  **Correction, recorded deliberately:** this CVE was cited earlier in this feature's
  lineage as evidence against **virtiofs**. That is a **misattribution** — the research
  says so directly (*"CVE-2026-24834 was about virtio-pmem's read-only enforcement"*).
  `virtio-pmem` + DAX and `virtio-fs` are different mechanisms; the CVE says nothing about
  the latter. **Do not repeat the misattribution.**

> **A second conflation, also corrected.** An earlier draft argued *"the reference
> implementation used virtiofsd — the option the most experienced team retreated from on
> this VMM."* That fuses two unrelated facts. Kata retreated from **`virtio-pmem`**, not
> from virtiofsd. And the reference implementation did **not** put its rootfs on virtiofs:
> `architecture.md:196` records that *"Other drives (code.ext4, deps.ext4) continue as
> block devices in the `disks` VmConfig section"* — only `attach_drive("output", …)` was
> intercepted and routed to virtiofs. **The reference already used exactly the split
> `[D5]` + `[D8]` now state.**

**Per-launch writability: `cp --reflink=auto` of the operator's rootfs, attached
read-write.** O(1) on XFS/btrfs, correct-but-slow elsewhere. Load-bearing for lifecycle
honesty: **a restart must get a fresh rootfs, not the crashed instance's mutations** —
otherwise backoff-restart silently becomes stateful and the operator's artifact is
mutated in place. Research Gap 2 flags reflink-vs-overlay as unmeasured; the measurement
belongs in Slice 00, not in a design assumption.

### `[D6]` netns / veth / mTLS — **slice 01–04 are mTLS-EXEMPT and Job-kind only, stated explicitly**

**Decision: `[vm]` + `[service]` is REJECTED with an honest error naming the missing
capability. `[vm]` + `[job]` and `[vm]` + `[schedule]` are in scope.**

This is stated, not implied. The reasoning:

1. **The interception layer is structurally blind to a guest.** `cgroup_connect4` and
   sockops intercept the **host** `connect()` syscall. A guest terminates TCP in its
   **own** kernel; the host sees virtio-net frames at a tap. This is GH
   [#222](https://github.com/overdrive-sh/overdrive/issues/222), which ADR-0069 folded in
   as the *staged guest-stack intercept adapter* of the one universal proxy. **It is
   open and unbuilt.**
2. **Probes cannot reach a guest** (`[G6]`): TCP/HTTP default to `127.0.0.1`, and the
   `Exec` mechanic joins the workload's cgroup and forks — there is no cgroup inside a
   guest. Service-kind depends on probes (J-OPS-004), so Service-kind VMs are gated on
   work that does not exist.
3. Building either inside this feature multiplies its size and produces a second
   feature's worth of mechanism — the opposite of thinner-but-live.

**Exempt must mean *visibly* exempt, not silently unencrypted.** `vision.md` design
principle 3 is *"security is structural, not configurable"*; a VM workload that cannot be
mesh-enrolled must not be reported as if it were. Job-kind is the honest boundary: a
run-to-completion VM job needs no mesh identity to be useful, so the feature ships a real
capability without overclaiming. The rejection error is a **feature**, not a gap — it
tells Ana exactly what is missing instead of deploying something that silently cannot be
reached.

> **Tracked.** #222 covers only the *mTLS intercept* half. Service-kind VM support also
> needs **(a) tap-in-netns provisioning** and **(b) a probe mechanism that reaches a
> guest** — both are GH
> [#257](https://github.com/overdrive-sh/overdrive/issues/257) (approved and filed
> 2026-08-02, closing the former `[B1]`). US-VM-6's rejection message cites **#222 and
> #257** so every named gap resolves to a real issue.

### `[D7]` The isolation claim — what this feature does and does not assert

**Decision (locked): the isolation posture is `KVM boundary + default-on seccomp +
Landlock + cgroup/netns confinement`, and there is `no jailer-equivalent chroot and no
PID namespace`.** That sentence is the claim. Nothing in this feature's artifacts,
code comments, or operator-facing text may assert more than it.

> **The claim is delivered in full only when Slice 03 lands — and the claim must not
> outrun it.** Items 5–6 (cgroup + netns placement, seccomp not weakened) arrive with
> Slice 01; items 1–3 (Landlock, uid/gid drop, rlimits) arrive with **US-VM-7 in Slice
> 03**. In the interval a VM's hypervisor is placed and seccomp-filtered but still root,
> Landlock-free and rlimit-unbounded. **Until US-VM-7 lands, no artifact, comment, error
> string, or operator-facing text may assert the Landlock / uid / rlimit half** — the
> in-flight posture is *"KVM + default-on seccomp + cgroup/netns confinement"* and that is
> all that may be said. This paragraph exists because precedent warning #6 is a claim
> shipping ahead of its code; a locked claim written at DISCUSS is exposed to exactly that
> failure for two slices, and the guard is a stated staging rule rather than good
> intentions.

**The precedent this exists to avoid.** The reference implementation's
`architecture.md:406` asserted *"VM isolation identical to Firecracker"* while its own
`research:430` recorded that Cloud Hypervisor has **no jailer** and prescribed
compensating hardening that was never built (intake precedent warning #6). The claim
outran the code, in the same repository, with the refuting evidence one file away.
**We must not repeat that claim.**

**What is true, and why it is worth stating.** A jailer is **not** the guest→host wall —
KVM is. It is the *second* wall: if a guest escapes into the VMM **process**, the jailer
bounds what the attacker inherits. Firecracker's jailer supplies `pivot_root` chroot ·
mount ns · optional PID ns · optional net ns · uid/gid drop · cgroup placement ·
`setrlimit` · `mknod` of `/dev/kvm` + `/dev/net/tun`. Cloud Hypervisor ships no jailer
(CH issue #5170 proposed one in Feb 2023 and closed without one). But CH supplies two
things Firecracker does not, or does differently:

- **Seccomp is ON BY DEFAULT**, with *per-thread* filters — vCPU, virtio-device,
  HTTP-API and main VMM threads each get a distinct filter. (Firecracker applies seccomp
  from the VMM itself, not from the jailer, so this is a peer capability, not a gap.)
- **Landlock is available opt-in** via `--landlock` — a kernel LSM under which *"the
  process cannot access any resources outside of the ruleset during its lifetime, even if
  it were compromised."* For the **filesystem half** of a jailer's job this is a *better*
  answer than chroot: no jail tree to construct, no `mknod`-ed device nodes to get wrong,
  and it holds **post-compromise**. Known limitation: xattr access remains permitted.
  Not on by default — this feature turns it on.

**And Overdrive already owns much of the rest.** `CgroupManager`
(create-scope → write-limits → enrol-PID, ADR-0026) is the cgroup placement; the shim's
`provision_and_inject_netns` already mints a per-workload netns that `ExecDriver` enters
via `setns(CLONE_NEWNET)`. The VMM is just another process to place and to launch into
that netns.

**Folded into this feature** (items 1–6, placement in § Scope assessment): `--landlock`
with a ruleset scoped to the VM's own kernel / rootfs / API-socket paths · uid/gid drop ·
`setrlimit` (`fsize`, `no-file`) · cgroup-scope placement · per-workload-netns placement ·
an assertion that `--seccomp` is never weakened to `false`/`log`.

**Left to #258**: PID-namespace isolation, **mount namespace / `pivot_root`**, device-node
exposure review, the written guest→VMM→host threat model, runtime EDD verification that
seccomp and Landlock are active on a *running* VM, **`virtiofsd` hardening —
UNCONDITIONAL**, and guest-agent attack surface (when
[#100](https://github.com/overdrive-sh/overdrive/issues/100) lands).

> **`virtiofsd` hardening is no longer conditional.** An earlier draft qualified it as
> *"conditional — this driver uses ext4 + virtio-blk, not virtiofs"*. `[D8]` puts
> `virtiofsd` on the roadmap as a real, supervised, per-VM host process, so the
> qualification is void. **#258 was amended 2026-08-02 to carry it unconditionally.**
>
> **The boundary between #258 and this feature, stated so neither side assumes the other
> covered it:**
>
> | Concern | Owner |
> |---|---|
> | `virtiofsd` **lifecycle** — spawn ordering, socket-readiness wait, supervision, teardown ordering, no leaked socket or orphan on any failure path, and **its death reaching `ExitKind` honestly** | **This feature — US-VM-9, Slice 04** |
> | `--sandbox` mode *selection* and its fail-closed rule (a spawn-argument property of the launch this feature owns, exactly as `[D7]` item 6 owns "seccomp never weakened" while the rest of the seccomp posture is #258's) | **This feature — US-VM-9, Slice 04** |
> | `virtiofsd` **posture** — its seccomp filter set, which uid it runs as, xattr/ACL surface, the daemon's own guest→daemon→host threat model | **#258** |

> **Why mount-ns moved to #258 — a shape argument, not a budget one.** A bare
> `unshare(CLONE_NEWNS)` is one syscall and buys ~nothing: the process gets a private
> *copy* of the mount table and still sees the entire host filesystem. The security value
> comes from the `pivot_root` + `MS_PRIVATE` propagation + bind-mounted artifacts +
> `mknod`/bind of `/dev/kvm` that follow it — which **is** the chroot half of the jailer,
> i.e. exactly what #258 owns and exactly what Landlock was selected *instead of*.
> Shipping the bare unshare would let us say "mount namespace isolation" while delivering
> the reference implementation's sin one level down. **RESOLVED — #258 was amended
> 2026-08-02 and now owns the mount namespace; this feature does not.** The argument above
> was accepted as stated. Nothing in this feature's artifacts may claim mount-namespace
> isolation for the hypervisor.
>
> **One asymmetry this creates, stated so it is not misread.** `[D8]` selects
> `--sandbox=namespace` for **`virtiofsd`**, which gives *that* process a mount namespace
> and a `pivot_root` into the shared directory — because virtiofsd ships its own sandbox
> and Cloud Hypervisor does not. **The `cloud-hypervisor` process still has neither.** A
> reader who sees "virtiofsd is namespace-sandboxed" must not infer the hypervisor is.

### `[D8]` Storage splits by ROLE — `virtio-blk` for the rootfs, `virtiofs` for volumes

**Decision (locked, = intake `I-6`): the mechanism is chosen by the storage's ROLE, not by
a global preference.**

| Role | Mechanism | Why |
|---|---|---|
| **Rootfs** — read-mostly, boots the guest | `ext4` image over **`virtio-blk`** | `[D5]`. CH's documented boot path; virtiofs-DAX (the thing that would justify FUSE in the boot path) does not exist on CH. |
| **Volumes** — the workload's output, shared writable storage | **`virtiofs`** (`virtiofsd`, `--memory shared=on`) | The right mechanism for a writable host↔guest share, and one of CH's three genuine differentiators (whitepaper §6, with CPU hotplug and Windows guests). |

**This is not a compromise between the research and the reference — both already say it.**
The research recommends `virtio-blk` for the **rootfs** and never evaluated volumes; the
reference implementation shipped block for `code.ext4` / `deps.ext4` and virtiofs for the
single writable `/output` share (`architecture.md:196`). The apparent conflict existed only
because the first DISCUSS pass **scoped the rootfs and never scoped volumes at all**, so
virtiofs fell out by omission. `[D8]` scopes volumes.

It is also the platform's stated direction:
[#97](https://github.com/overdrive-sh/overdrive/issues/97) plans `vhost-user-fs` for
persistent microVMs.

#### `[D8a]` The `[vm]` volume surface

```toml
[job]
name = "batch-render"

[vm]
kernel  = "/var/lib/overdrive/artifacts/vmlinux-6.18"
rootfs  = "/var/lib/overdrive/artifacts/render.ext4"
command = "/usr/bin/render"
args    = ["--frames", "120", "--out", "/output"]

[[vm.volume]]
source    = "/var/lib/overdrive/outputs/batch-render"   # host path — required
target    = "/output"                                    # guest mount point — required
read_only = false                                        # optional, default false
```

| Field | Surface | Rationale |
|---|---|---|
| `source` | **Operator** | The whole point — the operator names the host directory they will read afterwards. |
| `target` | **Operator** | The operator's own `command` writes to a path its code already knows. Deriving it would mean the platform picks a mount point the workload never writes to. **Contrast `[D1]`'s `cmdline`, which is deliberately NOT operator surface:** a wrong `cmdline` bricks the boot undiagnosably, whereas a wrong `target` fails *visibly* — the workload writes somewhere else and the host directory is empty. Different diagnosability, different surface call. That is the principled line, not an inconsistency. |
| `read_only` | **Operator**, default `false` | Writable is the named use case, but a read-only input mount is the natural sibling. Shipping writable-only would force write access on an operator who wanted a read-only dataset — a security downgrade by omission. **Enforced HOST-side — see `[D8g]`.** |
| virtiofsd socket path, tag, `--cache`, `--sandbox`, `--memory shared=…` | **Platform** | Allocation-scoped mechanism, not operator concerns. |

Zero volumes is the default and stays valid: a spec with no `[[vm.volume]]` is exactly
Slice 01's VM.

#### `[D8b]` `--memory shared=on` is CONDITIONAL — only volume-carrying VMs get it

**Decision: `shared=on` iff the spec declares at least one volume.**

`shared=on` is mandatory for any vhost-user backend (the backend must map guest memory), so
a volume-carrying VM has no choice. The question is whether *every* VM pays for it.

**Why conditional wins:**

1. **No cost without benefit.** A VM with no volumes gains nothing from a shared memory
   backing and pays whatever it costs. Slices 01–03's VMs have no volumes.
2. **It is not two code paths in any deep sense.** System constraint 4 already forbids a
   stateful builder: `Vmm::create` takes a `VmConfig` **value**. `shared` is one derived
   field (`!volumes.is_empty()`) with one construction site and one branch. The
   "two VM-config shapes" objection is an objection to a builder, which we do not have.
3. **Regression safety for already-landed slices.** A VM with no volumes boots
   byte-identically before and after Slice 04 lands. Making it unconditional would change
   the memory backing of every VM in slices 01–03 *after* they shipped.

**Reason 3 is the load-bearing one.** Stated explicitly because reason 1 presupposes a
non-trivial cost that has **not been measured yet**: `shared=on` backs guest RAM with a
shared mapping instead of private anonymous memory, which changes what the host can do with
those pages, but by how much is unknown. **Slice 00 P6 measures it** — same discipline
`[D5]`'s reflink cost gets from P4 — and **the measurement has a consequence in both
directions**: if the measured cost is negligible, `[D8b]` is re-opened toward unconditional
before Slice 04 is built, rather than reason 1 silently evaporating with nobody noticing.
Reason 3 survives either result and would carry the decision alone.

Reason 2 defuses the *construction* half of the two-config-shapes objection. The
*test-matrix* half is defused by US-VM-5 AC 5 (sizing parametrized over both backings) and
US-VM-8 AC 7 (the `[D7]` posture asserted on a volume-carrying allocation) — i.e. by
covering both shapes where they can actually diverge, not by asserting they cannot.

**The residual risk, named:** `shared=on` may interact with virtio-mem memory sizing —
US-VM-5's concern and GH
[#92](https://github.com/overdrive-sh/overdrive/issues/92)'s precondition. Ordering closes
it **inside** this feature: Slice 04 (volumes) lands **before** Slice 05 (resources), so
US-VM-5's sizing case runs on **both** memory shapes. See § Scope assessment.

#### `[D8c]` `--cache=never`

**Decision: `--cache=never`.** The reference implementation chose this and its reasoning is
sound and reusable: write-heavy, short-lived jobs, and it avoids cache-coherence complexity
between the host's view and the guest's. Two things make it *more* clearly right here than
there:

- Exactly **one** guest mounts each share (one `virtiofsd` per VM per `[D8d]`), so there is
  no multi-guest coherence benefit to trade away.
- **virtiofs DAX does not exist on CH** (`[D5]`), so a guest page cache over the share
  would be plain double-buffering — host page cache plus guest page cache for the same
  bytes — costing memory for no sharing.

`--cache` is **platform, not operator surface**. A read-heavy shared-dataset volume would
want `cache=auto`; that use case is **not in scope** and carries no forward pointer —
unstated knobs are out of scope by default.

#### `[D8d]` `--sandbox=namespace`, and it is NEVER silently downgraded

**Decision: `--sandbox=namespace` — virtiofsd's own default — and a host that cannot supply
it fails the allocation closed.**

**This is a correction of the reference implementation, not an inheritance from it.** Its
sandbox mode *silently* drifted from the researched `namespace` to the weaker `chroot`
across every downstream document, **with no justification recorded anywhere** — intake
precedent warning #6, the same failure shape as its *"VM isolation identical to
Firecracker"* claim. Deviating from a security default requires a stated reason; there was
none, so there is no drift to inherit.

Why `namespace` is worth taking rather than merely defaulting to:

- It gives `virtiofsd` a real mount namespace and a `pivot_root` into the shared directory
  — **precisely the jailer-shaped confinement `[D7]` could not give the hypervisor**,
  because Cloud Hypervisor ships no jailer and `virtiofsd` ships its own sandbox. Where the
  tool supplies it, take it.
- The alternative that actually shipped in the reference (`chroot`) is strictly weaker, and
  `none` presumes an external sandbox this platform does not have around `virtiofsd`.

**Fail-closed, not fall-back.** If `--sandbox=namespace` is unavailable (missing capability,
unsupported kernel, virtiofsd build without it), the allocation reaches `Failed` with a
named reason. **It never silently degrades to `chroot`** — a silent downgrade is verbatim
what the reference did, and writing the fail-closed rule is what prevents repeating it.
US-VM-9 owns this; `[D7]`'s boundary table assigns virtiofsd's deeper *posture* (its seccomp
set, its uid, xattr surface) to #258.

#### `[D8e]` Where `virtiofsd` sits, and what it does NOT widen

- **Same cgroup scope as the hypervisor.** Load-bearing: `cgroup_kill` at stop must reap
  the daemon too, or Slice 03's no-leak guarantee has a hole. **Stated consequence:**
  virtiofsd's memory counts against the workload's declared limit. That is honest
  accounting — it is the workload's storage daemon — but it is a real interaction with
  US-VM-5's sizing and is written down rather than discovered.
- **Same netns as the hypervisor.** It needs no network; an empty netns is stronger
  confinement, and it keeps the "no loose host process" property uniform. The CH↔virtiofsd
  channel is a **unix socket**, which is a filesystem object and crosses netns freely — so
  this placement costs nothing.
- **The volume source directory is NOT added to the hypervisor's Landlock ruleset.** Cloud
  Hypervisor never touches the source directory; it touches only the vhost-user *socket*.
  `virtiofsd` is the process that reaches the data. **Adding volumes therefore does not
  widen `[D7]`'s hypervisor confinement** — an explicit property, and an AC.

#### `[D8f]` Honest lifecycle sizing — this is not a small addition

The reference's `VirtiofsdManager` was **415 lines**: socket-path convention, spawn-before-
`vm.create` ordering, socket-readiness wait with timeout, `SIGTERM`→grace→`SIGKILL`, socket
unlink, and a `Drop` guard. Budgeted honestly rather than assumed cheap:

**RE-BUDGETED 2026-08-02 after peer review**, which found the first pass priced only the
host-side daemon lifecycle and omitted five rows — while presenting itself as "budgeted
honestly rather than assumed cheap." The corrected table:

| Concern | Reused from | New | Est. |
|---|---|---|---|
| Supervised host process, cgroup + netns placement | `VmDriver::start` (Slice 01) | — | 0 |
| `SIGTERM`→grace→`SIGKILL` | Slice 03's bounded-grace shutdown | — | 0 |
| Allocation-scoped socket path | `[D1]`'s platform-derived CH API socket path | — | 0 |
| No-leak-on-terminal hygiene | Slice 03 + the cgroup-leak discipline | — | 0 |
| **Ordering**: daemon up *and socket ready* before `vm.create`; torn down *after* the VM; no leak on any intermediate failure path | — | **Yes** | 2 d |
| **Socket-readiness wait with timeout** as a distinct named failure | — | **Yes** | *(in the 2 d above)* |
| **Two-directional exit honesty** (`[D3]`, constraint 9) | the rule | **Yes — the classification** | 1 d |
| **`[[vm.volume]]` array-of-tables parse + validation** *(omitted in the first pass)* | the `[vm]` parse surface | **Yes** | 0.5 d |
| **`VmConfig` volume payload + the derived `shared` flag** *(omitted)* | the `VmConfig` value shape | **Yes** | 0.5 d |
| **Guest-side mount in `overdrive-init` + refuse-to-exec + host→guest tag/target protocol** (`[D4]` amendment, `[D8g]`) *(omitted — the critical one)* | the agent's existing vsock framing | **Yes** | 1 d |
| **Host-side `read_only` export enforcement** (`[D8g]`) *(omitted)* | — | **Yes** | 0.5 d |
| **Failure vocabulary** — four new `TransitionReason` variants | Slice 02's shape | **Yes** | 0.5 d |
| **Tier-3 harness** — round-trip, read-only, mid-run kill, teardown, no-volume regression, mount failure | Slice 03's harness | **Yes** | 1.5 d |
| **A possible SECOND rkyv envelope bump** if `[[vm.volume]]` reaches the persisted aggregate and Slice 01's V2 did not anticipate it *(omitted)* | `[G4]`'s procedure | **Conditional** | 0–0.5 d |

**RE-SIZED: 6–9 days** (was 4–6), US-VM-8 ≈ 3.5–5 d, US-VM-9 ≈ 2.5–4 d. **Slice 04 is now
the largest slice after the walking skeleton, and its upper bound meets Slice 01's.** It is
deliberately not folded into that skeleton, which is 5–8 days on its own.

> **This re-size makes the lift question live rather than theoretical.** See § Scope
> assessment: the trigger is now a number, not a judgement call.

#### `[D8g]` The GUEST side — who mounts, and where `read_only` is enforced

Added 2026-08-02 after peer review. `[D8a]`–`[D8f]` scoped the **host** half of volumes
rigorously and left the **guest** half unscoped — *"and mounts the share in the guest at
`target`"*, passive, no subject. **That is structurally the same omission as the one this
whole amendment corrects** (`I-6` fell out because DISCUSS scoped the rootfs and never
scoped volumes), reproduced one level down. Scoping it:

**The mount owner is `overdrive-init`** — see the `[D4]` amendment above. PID 1 mounts each
declared volume at its `target` before exec'ing the command, and **refuses to exec if a
required mount fails.** There is no other candidate: a secondary virtiofs share is not
auto-mounted by the guest kernel, and PID 1 is the only in-guest process Overdrive controls.

**`read_only` is enforced HOST-side, not guest-side.** This is a decision, not a detail:

- A guest-side mount option (`-o ro`) is **guest-cooperative** — an uncooperative guest
  remounts read-write and the control is void. It would be worthless against precisely the
  untrusted workload `[D7]` and US-VM-7 are written for.
- Host-side enforcement (the daemon serving that share exports it read-only) holds
  regardless of what the guest does, which is what makes the `[D8a]` "security downgrade by
  omission" framing honest rather than decorative.
- **The guest-side `-o ro` is applied as well**, as an ergonomic guard that surfaces the
  error at the write rather than at the syscall boundary — but it is **not** the boundary,
  and no artifact may describe it as one.

**Claim discipline, per `[D7]`'s precedent:** if DESIGN finds host-side read-only export
unavailable, the honest move is to **strike the security framing from `[D8a]` and say
plainly that `read_only` is an ergonomic guard against accidental writes, not a boundary**
— never to keep the framing and ship the guest-side version. That inversion is precedent
warning #6 verbatim, which is why the fallback is written down before it can be needed.

**Consequence for the failure vocabulary:** a required mount that fails — bad tag, missing
mount point, a BYO kernel without virtiofs support, a read-only export the guest cannot
mount — is **one named `TransitionReason`**, and the allocation reaches `Failed` without
ever reaching a completed terminal state. US-VM-8 owns it.

#### Not in scope for `[D8]`

`overdrive-fs` chunk store, content-addressed volumes, volume lifecycle independent of the
allocation, multi-VM shared volumes, and `vhost-user-blk`. Persistent-microVM storage is
[#97](https://github.com/overdrive-sh/overdrive/issues/97); this is a host-directory share
scoped to one allocation.

---

## `[REF]` Scope assessment (Elephant Carpaccio gate) — **OVERSIZED as one unit; split applied**

Run before journey/story investment. **Four oversized signals fired.**

| Signal | Verdict |
|---|---|
| >10 user stories | **YES** unsplit — the spec surface, dispatch, VMM port, adapter, agent, readiness, exit, failure vocabulary, lifecycle, resources, and networking each want a story |
| >3 bounded contexts / modules | **YES — five**: `overdrive-core` (traits + aggregate + rkyv envelope), `overdrive-worker` (`VmDriver`), `overdrive-control-plane` (composition root + action shim), `overdrive-sim` (`SimVmm`), **plus a new guest-agent crate** |
| Walking skeleton >5 integration points | **YES — eight**: spec parse → rkyv intent → reconciler → action shim → driver dispatch → `Vmm` → CH process → guest agent |
| >2 weeks effort | **YES** |
| Multiple independently-shippable outcomes | **YES** — (a) a VM boots and exits honestly, (b) failures are diagnosable, (c) lifecycle converges, (d) **the workload's output reaches the operator**, (e) resources size the VM, (f) VMs join the mesh |

### The split applied

**Two cuts, in order:**

1. **Scope cut (the large one): `[vm]` + `[service]` is out** — `[D6]`. This removes
   tap-in-netns provisioning, guest-reachable probes, and the #222 guest-stack intercept
   adapter — roughly half the mechanism — and leaves a boundary that is honest rather
   than arbitrary. Outcome **(f)** becomes a separate, independently-drivable feature.
2. **Carpaccio cut: six slices**, each closing a loop through `overdrive serve` +
   `overdrive deploy`, ordered so the riskiest assumption dies first.

| # | Slice | Goal (one line) | Size |
|---|---|---|---|
| 00 | `spike-ch-boot-and-vsock` | Does the pinned 6.18 kernel boot under CH from an ext4 `virtio-blk` rootfs, does vsock carry a beacon **from inside a netns**, do the `[D7]` confinement flags compose with a real boot, and does **`virtiofsd` + `--memory shared=on` compose with both** — on the Lima dev kernel **and** the appliance kernel? | 3–4 d |
| 01 | `vm-job-boots-and-exit-code-is-honest` | `overdrive deploy` a `[vm]` + `[job]` spec → the guest runs → its **real** exit code reaches `workload describe`. **Walking skeleton.** | 5–8 d |
| 02 | `boot-failure-vocabulary` | A VM that fails to boot says **why**, in operator language, with a fix. | 2–3 d |
| 03 | `stop-restart-and-vmm-death` | `job stop` and crash-restart converge for VMs; an unreported VMM death is `Crashed`, never `CleanExit`; **the hypervisor process is confined** (`[D7]` items 1–3). | 4–6 d |
| **04** | **`vm-writes-output-the-operator-can-read`** | **A `[vm]`+`[job]` workload writes a file and the operator reads it on the host — and the storage daemon's death is classified as honestly as the hypervisor's (`[D8]`).** | **6–9 d** |
| 05 | `resources-size-the-vm` | `[resources]` drives vCPU count and memory — the #92 CPU-hotplug precondition. | 2 d |

**Slice 04 is new (2026-08-02) and Slice 05 is the former Slice 04, renumbered.** The
volume slice is placed *before* resources on outcome impact, not on effort — see
§ Priority rationale. It is **not folded into Slice 01**, which is already the over-budget
walking skeleton, nor into Slice 03, whose subject is the *hypervisor* process.

**Why volumes is one slice with two stories rather than one story.** US-VM-8 (the
capability) and US-VM-9 (the daemon's lifecycle honesty) are each right-sized on their own,
but they **must land together**: a slice that ships working volumes while a `virtiofsd`
death is misclassified is a slice whose deliverable is a lie — the same test that rejected
Slice 01's three candidate sub-splits. Two stories, one slice, one landing. This mirrors
Slice 03 exactly (US-VM-3 classifies the hypervisor's death, US-VM-4 ends it deliberately,
US-VM-7 bounds it while alive).

**Slice 00 is mandated by precedent warning #2**, not optional: the reference
implementation rated its equivalent spike `HIGH (blocker)` / *"Blocks all CH work"*, then
wrote 43 KB of implementation on top of it without running it. Governed by
`.claude/rules/spike.md` — throwaway code in gitignored `spike-scratch/`, run for real
under Lima, binary verdict.

**Slice 01 deliberately exceeds the 1–3 day norm and that is correct.** It is the walking
skeleton — the minimum that connects every activity — and the sub-splits were considered
and rejected as *dead* rather than *thin*:

- *"Land the `[vm]` parse + rkyv bump + dispatch first, adapter later"* → ships a spec
  the platform accepts and can never run: infrastructure wearing a green test suite,
  exactly CLAUDE.md's named anti-pattern.
- *"Boot the VM first, add the agent later"* → the interim state **is** the reference
  implementation's lie (`[D2]`/`[D3]`): `Running` on a 2xx and `CleanExit` from the VMM's
  exit code. A slice whose deliverable is a lie is not a thinner slice.
- *"`SimVmm` first"* → sim is not a production path.

There is no cut of the spine that leaves both halves live. The honest move is to name
Slice 01 as the largest, front-load its risk into Slice 00, and keep every **other**
slice thin.

### Where the `[D7]` confinement items land — and why

The six items are properties of **how `VmDriver` spawns the VMM**, so the naive placement
is "all of them in Slice 01's `start` path." Rejected: Slice 01 is already the
over-budget walking skeleton, and a standalone hardening slice is unavailable (the
slice-composition gate forbids a slice whose every story is `@infrastructure`). The
deciding question is instead **which items are inherent to writing `VmDriver::start` at
all**, and which are genuinely additive.

| Item | Lands in | Why there |
|---|---|---|
| **5.** cgroup scope + per-workload netns placement | **Slice 01** (US-VM-1 ACs) | **Inherent, not additive.** `ExecDriver::start` is already create-scope → write-limits → open-netns-FD → spawn-with-`setns`; `VmDriver::start` must have the same four steps or the VMM is uncontained and `cgroup_kill` has nothing to kill at stop (Slice 03). Sharper: `provision_and_inject_netns` is gated on `mtls_worker.is_some()`, **not on driver type** — on the mTLS-composed production boot *every* alloc including a VM one is handed `spec.netns`. A `VmDriver` that ignores it leaves a provisioned-then-torn-down netns the VMM never entered. Marginal cost ≈ 0; reuses `CgroupManager` and the `ExecDriver` `setns` shape verbatim. |
| **6.** `--seccomp` never weakened to `false`/`log` | **Slice 01** (US-VM-1 AC) | A constraint on argv Slice 01 already constructs. CH's default is on; the requirement is *not passing the flag that turns it off*. Placing it later would license Slice 01 to write `--seccomp false` and a later slice to remove it. Zero mechanism. |
| **1.** `--landlock` + scoped ruleset | **Slice 03** (US-VM-7) | Genuinely additive confinement. Independently demonstrable, independently failable. |
| **2.** uid/gid drop | **Slice 03** (US-VM-7) | Same. |
| **3.** `setrlimit` (`fsize`, `no-file`) | **Slice 03** (US-VM-7) | Same. |
| **4.** mount namespace | **NOT this feature — GH #258** | Bare `unshare(CLONE_NEWNS)` is a hollow claim; the real thing is `pivot_root` + device-node work, i.e. the chroot half of the jailer that Landlock was chosen instead of. See `[D7]`. **#258 needs amending.** |

**Items 1–3 form one story (US-VM-7) in Slice 03**, not three: they share a single Tier-3
observation harness against the live VMM (`/proc/<pid>/status`, `/proc/<pid>/limits`,
`/proc/<pid>/cgroup`, `/proc/<pid>/ns/net`) and one fail-closed decision. Slice 03 is the
right home on subject matter, not convenience — it is already "the VMM process as a
managed host process" (stop it, kill it, classify its death); bounding that same process
belongs with it. The composition gate holds: Slice 03 also carries US-VM-3 and US-VM-4,
both operator-visible.

**Two integration risks are pushed into Slice 00 rather than discovered in Slice 01/03**,
because both can invalidate a spawn path that is already built:

1. **vsock from inside a netns.** Item 5 puts the VMM in a per-workload netns while the
   host-side beacon listener is in the host netns. AF_VSOCK's netns behaviour is
   kernel-version-dependent — if the beacon cannot cross, `[D2]`'s entire Running gate
   breaks. Folded into Slice 00 **P2**.
2. **Confinement vs. a real boot.** A uid-dropped `cloud-hypervisor` still needs
   `/dev/kvm`; a Landlock ruleset that misses a path CH needs fails opaquely. New Slice
   00 **P5** proves the flags compose with a boot *before* Slice 01 depends on them.

**Verdict after the `[D7]` fold: still right-sized — 5 slices, 7 stories, 4 modules + 1 new
crate.** No signal that was clear before the fold had flipped. The `[vm]`+`[job]`-only
scope, the five-slice structure, and `J-OPS-003` traceability were all intact — the fold
added one story to an existing slice and two probes to an existing spike.

| Slice | Before fold | After fold | What moved |
|---|---|---|---|
| 00 | 1–2 d | **2–3 d** | Two added probes (P5, extended P2). No production code. |
| 01 | 5–8 d | **5–8 d — band unchanged** | Items 5–6. See the cost note below. |
| 02 | 2–3 d | 2–3 d | Unchanged. |
| 03 | 2–3 d | **4–6 d** | US-VM-7 (items 1–3). Now the second-largest slice. |
| 04 (now 05) | 2 d | 2 d | Unchanged. |

> **Slice 01's cost note — argued, not asserted.** Peer review challenged "unchanged" as
> under-argued, correctly. Split by half:
> **Cgroup placement was already budgeted** — intake I-2 assigns *"cgroup placement of the
> VMM process, netns entry"* to the driver, and the slice brief's Driver bullet predates
> this fold. Genuinely zero.
> **Netns entry is newly-*established* as required** — it follows from a grounding this
> amendment introduced (`provision_and_inject_netns` gates on mTLS composition, not driver
> type), so it is new work, not merely a newly-written AC. Its *code* cost is near zero: it
> is the `ExecDriver` shape verbatim — pre-open the FD, `setns(CLONE_NEWNET)` in a
> `pre_exec` hook (`driver.rs:389-397`, `:449-465`) — copied, not designed.
> **The genuinely new cost is test-side** — the `/proc` observation harness and PID
> resolution via the allocation's `cgroup.procs`, roughly half a day.
> That fits inside a band already three days wide, so the band does not move — but it is
> **not free**, and DELIVER should not be told it is.

### Scope re-assessment after `[D8]` (volumes) — **still right-sized; at the upper edge**

Re-run against the same five oversized signals, with volumes now in scope.

| Signal | Threshold | Now | Verdict |
|---|---|---|---|
| User stories | >10 | **9** (US-VM-1…9) | **PASS** |
| Bounded contexts / modules | >3 | **5** — unchanged: `overdrive-core` (`[[vm.volume]]` parse + `VmConfig`), `overdrive-worker` (`VmDriver` + the `virtiofsd` supervision), `overdrive-control-plane`, `overdrive-sim`, + the guest-agent crate. **`[D8]` adds no module** — the daemon is supervised from the same place the hypervisor is. | **PASS (no change)** |
| Walking-skeleton integration points | >5 | **8 — unchanged.** Volumes are **not** in the skeleton; Slice 01's VM declares no volume. | **PASS (no change)** |
| Effort | >2 weeks | **~22–32 d across six slices** (was ~15–23 d across five) | **Signal fires — resolved by slicing, as before** |
| Independently-shippable outcomes | >1 | **5 in-scope + 1 out** | **Resolved by the six-slice cut** |

**Verdict: right-sized as a sliced feature — 6 slices, 9 stories, 5 modules. Every slice
is independently drivable through `overdrive serve` + `overdrive deploy`, and the walking
skeleton is untouched.**

**Stated honestly: the feature is at its upper edge, and `[D8]`'s re-budget pushed it
further than the first pass claimed.** `[D8]` is the largest single addition since the
original split — it grows the total by roughly **7–9 days** (not the 5–6 the first pass
asserted, which had omitted the guest-side mount, the parse surface, the `VmConfig`
payload, host-side `read_only` enforcement and a possible second rkyv bump) and takes the
story count from 7 to 9.

**Three guardrails follow. They are commitments with a named owner and a named checkpoint,
because a trigger nobody is assigned to fire is not a gate.**

1. **A seventh slice, or a tenth story, splits the feature in two.**
2. **Slice 04 lifts out if it exceeds 9 days** — its stated upper band, not a judgement
   call. It depends on Slices 01–03 and nothing depends on it (Slice 05 is independent; the
   04→05 ordering is a priority call, not a dependency), so lifting requires **no**
   restructuring of the remainder.
3. **A trigger that this wave cannot adjust:** total feature effort above **35 days**, or
   **any single slice exceeding its stated upper band by more than 50%**. Guardrails 1 and 2
   are both measured in units this wave controls — the same amendment that added two stories
   also folded three confinement items into *one* story for a legitimate reason, which shows
   story count is adjustable. Guardrail 3 is not.

**Owner and checkpoint: DESIGN re-runs all three at the DESIGN handoff and returns a blocker
if any fires.** Not "someone notices later under delivery pressure."

> **Reconciling two claims that would otherwise contradict each other.** The priority
> rationale rates volumes **Value 5** on the grounds that *"a `[vm]`+`[job]` workload that
> cannot write output anywhere is not a batch workload — it is a number"*, while guardrail 2
> pre-authorises lifting Slice 04 out entirely. Both can hold, but only with the consequence
> stated: **if Slice 04 lifts, this feature's honest deliverable is a VM class that computes
> but cannot deliver** — trustworthy, diagnosable, correctly sized, and unable to produce an
> artifact. That is a legitimate thing to ship *as a delivery sequence*; it is **not** a
> licence to describe the VM class to operators as usable for batch work. **If Slice 04
> lifts, US-VM-8 and US-VM-9 become a hard prerequisite before any operator-facing material
> presents `[vm]` + `[job]` as production-ready.** Lifting changes when volumes land, never
> whether the class needs them.

**Why `[D8]` is not folded into an existing slice.** Slice 01 is the over-budget walking
skeleton; adding a supervised sidecar to it repeats the mistake the original split was
made to avoid. Slice 03's subject is *the hypervisor process*, and a second process with
its own ordering and failure vocabulary is a different subject. Slice 05 is 2 days of
derivation logic. None of the three is a home; volumes is its own slice.

---

## `[REF]` Story map

**Backbone** (Ana's end-to-end path, chronological):

| Declare a VM workload | Deploy it | Watch it come up | Read how it ended | Collect its output | Size it |
|---|---|---|---|---|---|
| `[vm]` table parses (US-VM-1) | dispatch reaches the VM driver (US-VM-1) | `Running` means the guest booted (US-VM-1) | real guest exit code (US-VM-1) | — *(the skeleton declares no volume)* | — |
| `[vm]`+`[service]` honestly rejected (US-VM-6) | boot failure says why (US-VM-2) | boot deadline → `Failed`, not hung (US-VM-2) | unreported VMM death → `Crashed` (US-VM-3) | guest writes land in Ana's host directory (US-VM-8) | `[resources]` → vCPU/mem (US-VM-5) |
| `[[vm.volume]]` names source + target (US-VM-8) | | the hypervisor runs confined (US-VM-7) | stop + restart converge (US-VM-4) | a read-only volume refuses guest writes (US-VM-8) | |
| | | the storage daemon is ready before the guest boots (US-VM-9) | a dead storage daemon is never a clean exit (US-VM-9) | | |

**Walking skeleton** (the horizontal line): US-VM-1 — one task from every activity except
output and sizing. `[vm]` parses → dispatch routes → the guest boots → the exit code is
real. **Volumes are deliberately below the skeleton line:** a VM that boots and reports an
honest exit code is a complete end-to-end loop without them, and putting a supervised
sidecar in the thinnest slice would fatten the skeleton — the named anti-pattern.

### Priority rationale

| Priority | Slice | Why this order |
|---|---|---|
| 1 | 00 (spike) | **Riskiest assumption first.** If the pinned kernel does not boot under CH from ext4/virtio-blk, or vsock does not carry the beacon, every later slice is built on sand — precedent warning #2 is exactly this failure. Cheapest possible place to learn it. |
| 2 | 01 (skeleton) | Only slice that closes the loop at all; every other slice is an increment on it. Also the only one that changes `lib.rs:1422` — the feature's pass/fail bar. |
| 3 | 02 (failure vocabulary) | Highest outcome impact per day. Without it every VM failure is `DriverInternalError { detail: <raw string> }` (`[G6]`) — Ana's exact stated frustration, "a diagnosis that requires reading source." |
| 4 | 03 (lifecycle + confinement) | Restores parity with the exec driver's already-trusted stop/restart contract, and bounds the hypervisor process (`[D7]` items 1–3). Below 02 because a workload that fails opaquely is worse than one that stops bluntly. Confinement rides here rather than in 01 because Slice 01's spawn path already carries the *inherent* half (cgroup + netns + seccomp-not-weakened); what remains is additive and independently failable. |
| **5** | **04 (volumes)** | **A `[vm]`+`[job]` workload that cannot write output anywhere is not a batch workload — it is a number.** Slices 01–03 deliver a trustworthy exit code; Slice 04 is what makes the class *produce* something, and it is the reference implementation's own core use case (`/output`). Placed **above** resources on outcome impact (Value 5 × Urgency 4 / Effort 4 = **5.0** vs resources' 4 × 2 / 2 = **4.0**), not below it on effort — effort-based ordering is the named anti-pattern. Two supporting reasons: it is the **recovered** intake decision `I-6`, and ordering a once-dropped decision last maximises the chance it is dropped again; and landing it before resources closes the `shared=on` × memory-sizing interaction **inside** this feature (US-VM-5 then runs on both memory shapes) rather than leaving it for #92 to discover. |
| **6** | **05 (resources)** | Unblocks GH [#92](https://github.com/overdrive-sh/overdrive/issues/92) (right-sizing / CPU hotplug), the commercial pillar that justifies Cloud Hypervisor at all — but it is worthless until 01–03 make VMs trustworthy, and its Tier-3 sizing case is *stronger* run after 04, because it can then be parametrized over both the private and the `shared=on` memory backing. **Still last for the reason it was always last**, not because volumes displaced it. |

> **Slices 04 and 05 are mutually independent.** Nothing in 05 depends on 04 and nothing in
> 04 depends on 05 — the ordering above is a *priority* call, not a dependency, and either
> can be resequenced without restructuring the other. The only thing lost by swapping them
> is the both-shapes sizing case in `[D8b]`.

---

## `[REF]` System constraints (cross-cutting; apply to every story)

1. **Vertical-slice bar.** Every story is driven through `overdrive serve` +
   `overdrive deploy`. No acceptance test installs, binds, programs, or supplies
   something `run_server` does not supply itself. A `Sim*` adapter injected at a port
   boundary is fine; hand-installing a missing production effect is not.
2. **`lib.rs:1422` must change.** Non-negotiable — `[G1]`, precedent warning #1.
3. **Single cut, no compatibility surface** (`feedback_single_cut_greenfield_migrations.md`):
   `DriverType::MicroVm` is deleted, `[vm]` lands, OpenAPI regenerates, and the false
   *"variants never change their wire form"* docstring is amended — **all in the same
   PR** as Slice 01.
4. **`Vmm` is a value-in, not a stateful builder** (I-2 shape caveat). One
   `Vmm::create(&VmConfig)` over a `VmConfig` **value**, making "boot before configured"
   *unrepresentable* rather than runtime-validated. Do not reproduce the reference
   implementation's `configure → set_boot_source → attach_drive → start` state machine
   policed by `validate_state_*`. **Cloud Hypervisor is the only implementor in scope** —
   designing for a hypothetical second adapter is speculative generality.
5. **DST reachability.** Every host effect the VM path introduces (process spawn,
   HTTP-over-unix API socket, vsock, tap) sits behind the `Vmm` port with a `SimVmm` in
   `overdrive-sim`; `dst-lint` bans those calls on `core`-class paths regardless.
6. **The isolation claim is `[D7]`, verbatim, everywhere.** CH ships no jailer (precedent
   warning #6), so the posture is **KVM boundary + default-on seccomp + Landlock +
   cgroup/netns confinement, and no jailer-equivalent chroot or PID namespace.** No
   artifact, code comment, error message, or operator-facing string may assert more —
   *"isolation identical to Firecracker"* is the named forbidden sentence. The
   jailer-equivalent remainder is GH
   [#258](https://github.com/overdrive-sh/overdrive/issues/258). **The claim is staged:
   until US-VM-7 lands in Slice 03, only the delivered half — *KVM + default-on seccomp +
   cgroup/netns confinement* — may be asserted anywhere.**
7. **Version floors must state what breaks.** Precedent warning #7: the reference
   implementation asserted CH ≥ 48.0 and virtiofsd ≥ 1.10 across six documents with **no
   stated reason anywhere**, and never built the check. If this feature declares a CH
   floor, it names the API or behaviour that is absent below it, or it declares none.
   `[D7]` supplies the first genuine candidate: **`--landlock` is a named CH flag with a
   first-shipped version**, and US-VM-7 fails closed when it is unavailable — so a floor
   declared on that basis states exactly what breaks below it. Slice 00 P5 measures it.
8. **The ~75 ms hotplug boot cost is accepted and quantified**, not discovered later. The
   reference implementation's research attributes ~75 ms of CH's boot time to the
   hotplug/vhost-user feature surface. It ran short-lived fire-and-forget jobs and paid
   the cost for nothing. Overdrive has Service-kind workloads, #92, and #96–#100 — it
   collects. **State the case as "CPU hotplug unblocks #92" (irrefutable), never as "CH
   has hotplug" (refutable: Firecracker shipped virtio-mem memory hotplug in v1.14.0,
   2024-12-17).
9. **A supervised sidecar's death is classified by the WORKLOAD's outcome, never by the
   sidecar's own exit status.** `[D3]`'s rule for the `cloud-hypervisor` process applies
   unchanged to every process this feature supervises on a workload's behalf —
   `virtiofsd` today (`[D8]`, US-VM-9), any future vhost-user backend tomorrow. Both
   directions are forbidden, and the reference implementation got **both** wrong: a clean
   sidecar exit must not be reported as a crash (its `wait()` treated *any* `virtiofsd`
   exit, including a clean one, as `VmmError::Crash` and force-killed the VM), and a
   sidecar that dies mid-run must not let the workload's ending be reported as a clean
   exit. A sidecar is not the workload; its exit status is not evidence about the
   workload's.
10. **Storage mechanism is chosen by ROLE (`[D8]`), and no artifact says otherwise.** No
    document, comment, or message in this feature may state or imply that virtiofs was
    *rejected* — the rootfs decision (`[D5]`) is scoped to the rootfs. **CVE-2026-24834
    may not be cited as evidence about virtiofs**: it concerns `virtio-pmem` + DAX, and
    citing it against virtiofs is a misattribution this feature has already made once and
    corrected.**

---

## `[REF]` User stories

> All stories trace to **J-OPS-003** (extended). Each non-`@infrastructure` story carries
> an Elevator Pitch whose "After" names a real operator entry point and concrete
> observable output. ACs are embedded and testable.

<!-- markdownlint-disable MD024 -->

### US-VM-1 — A `[vm]` workload boots under Cloud Hypervisor and its real exit code reaches the operator

**Job:** `J-OPS-003` · **Slice:** 01 (walking skeleton) · **MoSCoW:** Must

#### Elevator Pitch

- **Before:** Ana has a workload that needs its own kernel. Overdrive can only run host
  processes — `[vm]` is not a table the parser knows, and the composition root
  (`lib.rs:1422`) constructs exactly one `ExecDriver`. There is no path, and no error
  that explains one is missing.
- **After:** Ana runs `overdrive deploy render.toml` with a `[vm]` table naming a kernel
  and an ext4 rootfs → a real Cloud Hypervisor VM boots, her `command` runs in the guest,
  and `overdrive workload describe render` shows
  `Terminated / Completed{exit_code: 7}` when the guest exited **7** — not `0`, not
  "succeeded".
- **Decision enabled:** Ana can decide whether the run succeeded, and the restart/backoff
  reconciler can decide whether to retry — both from a signal that came from **inside**
  the guest rather than from the hypervisor's own exit status.

#### Problem

Ana Moreno needs to run a workload that cannot be a host process — it needs a different
kernel or hardware-level isolation. Overdrive advertises VMs as first-class
(`vision.md` principle 4) but has no VM path at all: no `[vm]` table, no VM driver, and a
composition root that can only ever hand back an `ExecDriver`.

#### Who

Ana Moreno, Overdrive platform engineer | single-node dev host, control plane and worker
co-located | reasons in intent-vs-actual and treats `workload describe` as a promise.

#### Solution

A `[vm]` driver table (`kernel`, `rootfs`, `command`, `args`) parsed as one of "exactly
one driver table"; a `Vmm` port trait with `CloudHypervisorVmm` and `SimVmm`; a
`VmDriver: Driver` composing over `Arc<dyn Vmm>`; driver dispatch at the composition
root; and a static PID-1 `overdrive-init` that beacons ready, execs the command, and
reports the real exit status over vsock. Job-kind only.

#### Domain Examples

##### 1. Happy path — a batch render completes

Ana deploys `render.toml`: `[job] name = "batch-render"`, `[vm] kernel =
"/var/lib/overdrive/artifacts/vmlinux-6.18"`, `rootfs = ".../render.ext4"`, `command =
"/usr/bin/render"`, `args = ["--frames", "120"]`. The VM boots, `overdrive-init` beacons
ready, the render runs 40 s and exits `0`. `overdrive workload describe batch-render`
shows `Terminated / Completed{exit_code: 0}`.

##### 2. Non-zero exit — the code survives the boundary

Ana deploys `checksum.toml` whose guest command is `/usr/bin/verify` and the guest exits
**7**. `overdrive workload describe checksum` shows `Failed` with `exit_code: 7`. The
host `cloud-hypervisor` process exited `0` — and that number appears nowhere.

##### 3. Error — a rootfs with no working init never reports Running

Ana deploys `broken.toml` pointing at an ext4 image whose init is absent. The kernel
boots, no beacon arrives, the boot deadline expires. The allocation goes
`Pending → Failed` and **never passes through `Running`**.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A VM workload runs to completion and its exit code reaches the operator
  Given Ana has a kernel and an ext4 rootfs on the host whose guest command exits 0
  And she has written render.toml declaring [job] and [vm] naming those artifacts
  When she runs "overdrive deploy render.toml" against a running "overdrive serve"
  Then a Cloud Hypervisor VM boots and runs her command in the guest
  And "overdrive workload describe batch-render" shows Terminated with exit code 0

Scenario: A non-zero guest exit code is reported, not the hypervisor's
  Given Ana has deployed a VM workload whose guest command exits 7
  When the guest command finishes and the VM shuts down
  Then "overdrive workload describe" shows the allocation Failed with exit code 7
  And the exit code reported is the guest's, not the cloud-hypervisor process's

Scenario: A guest that never starts is never reported as Running
  Given Ana has deployed a VM workload whose rootfs has no working init
  When the boot deadline elapses without the guest reporting ready
  Then the allocation transitions from Pending to Failed
  And the allocation never passes through Running

Scenario: A VM workload is deployed through the same verb as a process workload
  Given Ana already deploys process workloads with "overdrive deploy <spec>"
  When she deploys a spec whose driver table is [vm] instead of [exec]
  Then the workload is accepted and scheduled with no new verb and no new flag

Scenario: The platform contains the hypervisor it started on Ana's behalf
  Given Ana has deployed a VM workload on a node running the production composition
  When the allocation reaches Running
  Then the hypervisor is inside that allocation's resource scope and its network namespace
  And the hypervisor's syscall surface is filtered rather than left at the host default
```

#### Acceptance Criteria

Derived from the five scenarios above. Engineering constraints that are **not** UAT-derived
live in their own block below — they are binding, but they are not acceptance criteria.

- [ ] *(Scenarios 1, 4)* Driven end-to-end through `overdrive serve` + `overdrive deploy` —
      **no acceptance test installs, binds, programs, or supplies anything `run_server` does
      not supply itself**; no new verb and no new flag.
- [ ] *(Scenario 4)* A spec with `[vm]` and no `[exec]` parses; a spec with **both** or
      **neither** is rejected with an "exactly one driver table" error naming both tables.
- [ ] *(Scenarios 1, 2)* The exit code surfaced in `workload describe` is the **guest's**,
      proven by a Tier-3 case using a non-zero, non-trivial code (e.g. 7) while the host
      `cloud-hypervisor` process exits 0.
- [ ] *(Scenario 3)* An allocation reaches `Running` only after the guest agent's ready
      beacon; a rootfs with no working init reaches `Failed` **without** passing through
      `Running`.
- [ ] *(Scenario 5)* **`[D7]` item 5 — the hypervisor process is placed, not loose.** For a
      running VM allocation: `/proc/<vmm-pid>/cgroup` resolves to that allocation's workload
      scope, and `/proc/<vmm-pid>/ns/net` is the inode of `/var/run/netns/<spec.netns>` and
      **not** the host netns. Read from `/proc` for the live hypervisor — not asserted
      against the placement call.
      **The Tier-3 case MUST run against an mTLS-composed `overdrive serve`** (the
      production composition, `dataplane_override` unset) so that `spec.netns` is
      known-supplied and the netns half of this criterion cannot pass vacuously. An
      mTLS-uncomposed boot leaves `spec.netns = None` (`[G6]`) and would satisfy a
      conditionally-worded assertion with **zero placement code written** — the
      GH #248 / ADR-0074 trap (`.claude/rules/development.md` § "Ground the premise"),
      reproduced here on purpose so DELIVER cannot walk into it.
- [ ] *(Scenario 5)* **`[D7]` item 6 — seccomp is never weakened.** **The driver constructs
      the seccomp argument explicitly rather than relying on CH's default**, so the negative
      property has a real mutation site. A mutation flipping that argument to `false` **or**
      `log` must be killed **by an assertion over the constructed argument itself** — not by
      the `/proc` read. *`/proc/<vmm-pid>/status`'s `Seccomp:` mode distinguishes off from on
      and so kills the `false` mutation, but **CH's `log` mode still installs a filter**, so
      the mode stays non-zero and `log` survives a `/proc`-only check.* The `/proc` read is
      retained as a runtime regression guard; the argv-level assertion is what makes the
      criterion complete. *(Both halves are stated because a "no code path constructs X"
      criterion over code that never mentions X has no mutation site at all — cargo-mutants
      cannot synthesise an argv element.)*

##### Resolving `<vmm-pid>`

Every `/proc/<vmm-pid>/…` assertion in this feature resolves the allocation's hypervisor PID
through **that allocation's cgroup scope `cgroup.procs`** — which item 5 is what guarantees.
Consequence, stated so it is not discovered later: **US-VM-1's cgroup placement is a
prerequisite for *verifying* US-VM-7's items 1–3**, a Slice-03-onto-Slice-01 verification
dependency, not merely a slice ordering.

#### Engineering Constraints (binding; Definition of Done, not UAT-derived)

- [ ] `crates/overdrive-control-plane/src/lib.rs:1422` no longer hardcodes a single
      `Arc::new(ExecDriver::new(...))`; the action shim routes to the driver the spec's
      table names. *(The mechanism — registry vs enum dispatch vs composite — is DESIGN's;
      the routing requirement is not.)* **This is the feature's pass/fail bar** — CLAUDE.md
      § "Build vertical slices through production entry points" + precedent warning #1.
- [ ] `DriverType::MicroVm` is deleted, `[vm]` is the only VM table, OpenAPI is
      regenerated, and the `traits/driver.rs:26-29` "wire form never changes" docstring is
      amended — same PR (single cut).
- [ ] **`JobEnvelope` V1 → V2** — required by `WorkloadDriver::Vm` (`[G4]`) — lands as a
      **single commit** via the six-step procedure, with a new golden-bytes fixture pinning
      V1 and a `From<JobV1> for JobV2` impl; existing fixtures untouched. **Not conditional
      — user-ruled 2026-08-02.**
- [ ] `SimVmm` exists in `overdrive-sim` and the VM path is reachable from Tier-1 DST.

#### Technical Notes

- `AllocationSpec` derives no serde/rkyv (`[G5]`) — adding artifact fields costs no
  schema evolution. `command`/`args` are reused for the in-guest entrypoint.
- 11 irrefutable `let WorkloadDriver::Exec(..) =` destructures become `match`es
  (compiler-enforced tripwires, ADR-0031:197).
- `ParseError::MissingExec` (`workload_spec.rs:743-745`) becomes an
  exactly-one-driver-table error; all three kind specs carry a non-optional `exec` today.
- ADR-0030 §6 pre-sanctioned per-driver-class spec types; ADR-0022 pre-committed the
  registry migration to "the second driver class" — **this is that moment.**
- **`spec.netns` arrives whether or not the VM uses it.** `provision_and_inject_netns`
  (`action_shim/mod.rs:839`) gates on `mtls_worker.is_none()`, **not on driver type** — so
  on the mTLS-composed production boot every alloc, VM included, is handed `spec.netns` /
  `host_veth` / `workload_addr` and gets a matching teardown at terminal. A `VmDriver` that
  ignores `spec.netns` therefore leaves a provisioned-then-destroyed netns nothing entered.
  Entering it is the `ExecDriver` shape verbatim (pre-open the FD, `setns(CLONE_NEWNET)` in
  a `pre_exec` hook — `driver.rs:389-397`). Job-kind VMs need no tap inside it; an empty
  netns is *stronger* confinement, not a gap. **`[D6]` mTLS-exemption means the guest is not
  mesh-enrolled — it does not mean the alloc has no netns.**
- **vsock must cross the netns boundary** for `[D2]`'s beacon to arrive once the VMM is
  placed. Kernel-version-dependent; **Slice 00 P2 settles it before this slice depends on
  it.**
- Cgroup placement reuses `CgroupManager` unchanged (`create_workload_scope` →
  `write_resource_limits` → `place_pid_in_scope`, ADR-0026 D9 ordering).
- Depends on Slice 00 PROMOTE.

---

### US-VM-2 — A VM that fails to boot says why, in operator language

**Job:** `J-OPS-003` · **Slice:** 02 · **MoSCoW:** Must

#### Elevator Pitch

- **Before:** Every VM failure lands in `TransitionReason::DriverInternalError { detail:
  <raw string> }` (`action_shim/mod.rs:234`) — the exec-shaped prefix table matches
  nothing a VM produces. Ana sees a hypervisor error string and has to read source to
  learn whether her kernel path was wrong, her rootfs was missing, or `cloud-hypervisor`
  is not installed.
- **After:** Ana runs `overdrive deploy render.toml` with a typo in the kernel path →
  `overdrive workload describe batch-render` shows a `Failed` reason naming **the kernel
  artifact and the path that was not found**, distinct from a missing rootfs, a missing
  `cloud-hypervisor` binary, and a boot that timed out.
- **Decision enabled:** Ana fixes the right thing on the first try — she knows whether to
  correct a path, install a binary, or investigate the guest.

#### Problem

Ana's stated frustration is *"a diagnosis that requires reading source instead of reading
the CLI."* A VM has more distinct pre-guest failure modes than a process does, and today
all of them collapse into one opaque string.

#### Who

Ana Moreno | deploying an unfamiliar VM artifact for the first time | no hypervisor
expertise assumed.

#### Solution

VM-shaped `TransitionReason` variants and a VM arm in `classify_driver_failure` (whose
`DriverType` parameter is unused today and exists precisely for this —
`action_shim/mod.rs:200`). Each distinct failure gets its own variant with its own
operator-facing message; none is a catch-all.

#### Domain Examples

##### 1. Kernel artifact not found

Ana typos `vmlinux-6.18` as `vmlinux-6.8`. `workload describe` reports the **kernel
artifact** missing at that exact path — not "rootfs", not "internal error".

##### 2. Hypervisor binary absent

Ana deploys on a host without `cloud-hypervisor` installed. The reason names the missing
hypervisor binary, distinct from any artifact problem.

##### 3. Boot deadline exceeded

Ana's rootfs is intact but its init hangs. After the boot deadline the allocation reports
boot-timeout with the elapsed deadline — distinct from a missing artifact, because
nothing was missing.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A missing kernel artifact is named precisely
  Given Ana has deployed a VM workload whose [vm] kernel path does not exist
  When the platform attempts to start the allocation
  Then the allocation is Failed with a reason naming the kernel artifact and the path
  And the reason is distinct from a missing rootfs

Scenario: A missing hypervisor binary is distinguished from a missing artifact
  Given the host has no cloud-hypervisor binary installed
  When Ana deploys a VM workload whose artifacts all exist
  Then the allocation is Failed with a reason naming the missing hypervisor binary

Scenario: A guest that hangs during boot reports a timeout, not a missing artifact
  Given Ana has deployed a VM workload whose rootfs init hangs forever
  When the boot deadline elapses
  Then the allocation is Failed with a boot-timeout reason naming the deadline
  And the allocation never passes through Running

Scenario: An unclassified hypervisor failure still carries its verbatim cause
  Given a VM start fails for a reason the platform does not have a variant for
  When Ana reads workload describe
  Then the reason carries the verbatim hypervisor error text
  And it is labelled as unclassified rather than presented as a known cause
```

#### Acceptance Criteria

- [ ] Distinct `TransitionReason` variants exist for: kernel artifact not found, rootfs
      artifact not found, hypervisor binary absent, boot deadline exceeded. **No two share
      a variant.** *(**US-VM-7 mints a fifth in the same shape** — confinement unavailable —
      when Slice 03 lands. Four is this slice's set, not the feature's ceiling; K3's target
      is "≥ 4 distinct", so the fifth extends the vocabulary rather than contradicting it.)*
- [ ] `classify_driver_failure` routes VM failures via its (currently unused)
      `DriverType` parameter; exec classification is unchanged.
- [ ] Each variant's operator-facing message names the artifact or resource and the
      actionable next step — verified by reading `workload describe` output, not by
      asserting on an enum.
- [ ] Genuinely unclassified failures fall through carrying the **verbatim** cause and are
      labelled unclassified — never presented as a known cause
      (`.claude/rules/development.md` § "Distinct failure modes get distinct error
      variants").
- [ ] Every case is produced by a real `overdrive deploy` against a real
      `overdrive serve`, not by constructing a `DriverError` in a test.

#### Technical Notes

- Mirrors the existing exec vocabulary (`ExecBinaryNotFound`, `ExecPermissionDenied`,
  `CgroupSetupFailed`) — same shape, VM nouns.
- The boot deadline is the same deadline `[D2]` uses to bound `VmDriver::start`.

---

### US-VM-3 — A VM's death is classified honestly: the hypervisor exiting 0 is not the workload succeeding

**Job:** `J-OPS-003` · **Slice:** 03 · **MoSCoW:** Must

#### Elevator Pitch

- **Before:** If the platform derives `ExitKind` from the host `cloud-hypervisor`
  process's exit status, a guest that boots, kernel-panics, and shuts down cleanly exits
  the VMM `0` — and Overdrive reports `Terminated / Completed`. Ana sees a successful
  batch job that never ran. The lie is undetectable from outside, so restart/backoff never
  fires.
- **After:** Ana's VM is killed by a guest kernel panic → `overdrive workload describe`
  shows `Failed` with a reason stating **the guest died without reporting** — and the
  restart/backoff reconciler retries it, because it correctly saw a crash.
- **Decision enabled:** Ana can trust that a green terminal state means the workload
  actually finished, so she can build on the result instead of re-verifying every run by
  hand.

#### Problem

This is the single most likely place for this feature to ship a lie. The reference
implementation shipped an acceptance criterion — *"job completion status is reported
correctly"* — that nothing in its design could satisfy, because its `wait()` observed the
**host** VMM process. Overdrive's `ExitEvent` contract, its restart/backoff reconciler
(`workload_lifecycle.rs:673-743`), and `workload describe` all consume that signal.

#### Who

Ana Moreno | relying on VM terminal state to gate downstream work and on backoff to
retry genuine crashes.

#### Solution

Classify the three situations distinctly (`[D3]`): agent-reported status is authoritative;
a VMM exit with **no** agent report is `Crashed` with a reason naming the unreported
death; an operator stop is `intentional_stop`. The host process's exit code is never the
source of `ExitKind`.

#### Domain Examples

##### 1. Guest panic — VMM exits 0, allocation is Failed

Ana's `batch-render` guest kernel-panics 3 s in. `cloud-hypervisor` exits `0`.
`workload describe` shows `Failed` with an unreported-guest-death reason, and backoff
schedules a retry.

##### 2. Host OOM kills the hypervisor

The `cloud-hypervisor` process for `checksum` is OOM-killed. No agent report arrived.
The allocation is `Failed`, not `Terminated / Completed`.

##### 3. Clean completion — the agent's word wins

`batch-render`'s guest command exits `0` and the agent reports it. The allocation is
`Terminated / Completed{exit_code: 0}` — and this is the **only** path to that state.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A guest kernel panic is a crash, not a clean completion
  Given Ana has deployed a VM workload whose guest kernel panics after boot
  When the hypervisor process exits with status 0
  Then the allocation is Failed with a reason naming the unreported guest death
  And the allocation is not Terminated with a completed condition

Scenario: A hypervisor killed by the host is a crash
  Given Ana has deployed a VM workload and its hypervisor process is killed by the host
  When the platform observes the hypervisor exit without an agent report
  Then the allocation is Failed
  And the restart and backoff behaviour matches a crashed process workload

Scenario: Only an agent-reported exit can produce a completed terminal state
  Given Ana has deployed a VM workload whose guest command exits 0 and reports it
  When the VM shuts down
  Then the allocation is Terminated with completed exit code 0

Scenario: An operator stop is not counted as a crash
  Given Ana has deployed a running VM workload
  When she stops it with the operator stop verb
  Then the allocation is Terminated as operator-stopped
  And no restart budget is consumed
```

#### Acceptance Criteria

- [ ] `ExitKind::CleanExit` for a VM allocation is produced **only** from an
      agent-reported guest exit status. No code path derives it from the
      `cloud-hypervisor` process's exit status.
- [ ] A VMM exit with no agent report yields `ExitKind::Crashed` with a reason naming the
      unreported guest death — proven by a Tier-3 case where the guest dies and the
      hypervisor exits **0**.
- [ ] An operator stop yields `intentional_stop: true` and consumes no restart budget.
- [ ] Restart/backoff behaviour for a crashed VM matches a crashed process workload
      (same reconciler, same ceiling, same backoff).
- [ ] Classification logic is covered by mutation testing per `.claude/rules/testing.md`
      § "Mutation testing" — a mutation collapsing the unreported-death arm into
      `CleanExit` **must be killed**.

#### Technical Notes

- Consumers: `exit_observer::classify` (`worker/exit_observer.rs:610-636`),
  `workload_lifecycle.rs:673-743`.
- The agent's report and the VMM's exit race; DESIGN pins the ordering so a reported exit
  is never overwritten by the subsequent VMM teardown.

---

### US-VM-4 — Stopping and restarting a VM workload converges like any other workload

**Job:** `J-OPS-003` · **Slice:** 03 · **MoSCoW:** Must

#### Elevator Pitch

- **Before:** The reference implementation's only shutdown was SIGKILL — no graceful
  path. A VM stopped that way loses in-flight writes and never runs shutdown handlers,
  and Ana has no way to tell a clean stop from a kill.
- **After:** Ana runs `overdrive job stop batch-render` → the guest is asked to shut down
  gracefully, and `overdrive workload describe` shows
  `Terminated / Stopped{by: Operator}` — the same terminal shape she already reads for
  process workloads.
- **Decision enabled:** Ana can stop and roll VM workloads with the verbs and terminal
  states she already knows, without learning a second lifecycle vocabulary per workload
  class.

#### Problem

Lifecycle parity is the whole promise of "one control plane, all workload types." A VM
that can only be killed, or whose restart reuses a mutated rootfs, breaks the convergence
contract J-OPS-003 already guarantees for processes.

#### Who

Ana Moreno | stopping and rolling workloads as routine ops.

#### Solution

`Driver::stop` for VMs requests a graceful guest shutdown, escalating to a hard kill after
a bounded grace period; restart gets a **fresh** rootfs copy (`[D5]`), never the crashed
instance's mutated one.

#### Domain Examples

##### 1. Graceful stop

Ana runs `overdrive job stop batch-render`. The guest receives a shutdown request, exits
its command, and the allocation reaches `Terminated / Stopped{by: Operator}`.

##### 2. Unresponsive guest escalates

`checksum`'s guest ignores the shutdown request. After the grace period the platform kills
the hypervisor; the allocation still reaches `Terminated / Stopped{by: Operator}` — an
operator stop, not a crash.

##### 3. Restart gets a clean rootfs

`batch-render` crashes after writing garbage into its rootfs. Backoff restarts it. The
new allocation boots from a **fresh copy** of Ana's original artifact, and Ana's artifact
on disk is byte-unchanged.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Stopping a VM workload reaches the same terminal state as a process workload
  Given Ana has a running VM workload
  When she runs the operator stop verb
  Then the guest is asked to shut down gracefully
  And the allocation reaches Terminated as operator-stopped

Scenario: An unresponsive guest is stopped within a bounded grace period
  Given Ana has a running VM workload whose guest ignores shutdown requests
  When she runs the operator stop verb
  Then the allocation reaches Terminated as operator-stopped within the grace period
  And it is not classified as a crash

Scenario: A restarted VM boots from a clean rootfs
  Given a VM workload crashed after modifying its rootfs
  When the platform restarts the allocation under backoff
  Then the new allocation boots from an unmodified copy of the operator's artifact
  And the operator's artifact file on the host is byte-unchanged
```

#### Acceptance Criteria

- [ ] `overdrive job stop` on a VM workload requests a graceful guest shutdown before any
      hard kill.
- [ ] An unresponsive guest is terminated within a bounded grace period and still lands
      `Terminated / Stopped{by: Operator}` — never a crash classification.
- [ ] A restarted VM allocation boots from a fresh copy of the operator's rootfs; the
      operator's artifact on disk is byte-identical before and after.
- [ ] Stop and restart are driven through `overdrive serve` + the operator verb, not a
      test-invoked `Driver::stop`.

#### Technical Notes

- The grace period is a platform constant, not operator surface, until a need appears.
- No leaked hypervisor processes or rootfs copies after terminal states — the same
  hygiene the cgroup-leak discipline enforces for exec workloads.

---

### US-VM-5 — `[resources]` sizes the VM's vCPUs and memory

**Job:** `J-OPS-003` · **Slice:** 05 *(renumbered from 04 when volumes became Slice 04)* ·
**MoSCoW:** Should

#### Elevator Pitch

- **Before:** A VM boots at whatever default the platform picks. Ana's `[resources]`
  block — which already governs process workloads and which the right-sizing reconciler
  (#92) reads — is silently ignored for VMs. A VM asking for 4 GB gets whatever the
  hypervisor default is.
- **After:** Ana deploys with `cpu_milli = 2000` and `memory_bytes = 2147483648` → the
  guest sees **2 vCPUs and 2 GiB**, observable from inside the guest and reflected in
  `overdrive workload describe`.
- **Decision enabled:** Ana can size a VM workload with the same block she already uses,
  and capacity planning stays one mental model across workload classes.

#### Problem

Two sources of truth for a VM's size would break the #92 right-sizing reconciler before
it is written — CPU hotplug is the reason Cloud Hypervisor was chosen at all (whitepaper
§14's commercial pillar), and it can only work against a single declared size.

#### Who

Ana Moreno | sizing workloads and planning node capacity.

#### Solution

Derive vCPU count from `resources.cpu_milli` and memory size from
`resources.memory_bytes`; `[vm]` never carries either.

#### Domain Examples

##### 1. Two vCPUs

`cpu_milli = 2000` → the guest reports 2 online CPUs.

##### 2. Sub-core request floors at one vCPU

`cpu_milli = 250` → the guest gets 1 vCPU (a VM cannot have a fractional CPU; the
platform floors at one rather than refusing).

##### 3. Memory is exact

`memory_bytes = 2147483648` → the guest reports approximately 2 GiB, and
`workload describe` reports the same declared figure.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Declared CPU translates to guest vCPUs
  Given Ana has deployed a VM workload declaring 2000 cpu_milli
  When the guest boots
  Then the guest reports two online CPUs

Scenario: A sub-core CPU request still yields a usable VM
  Given Ana has deployed a VM workload declaring 250 cpu_milli
  When the guest boots
  Then the guest reports one online CPU
  And the allocation reaches Running

Scenario: Declared memory is what the guest gets
  Given Ana has deployed a VM workload declaring 2147483648 memory_bytes
  When the guest boots
  Then the guest reports approximately 2 GiB of memory
```

#### Acceptance Criteria

- [ ] vCPU count is derived from `resources.cpu_milli` with a documented rounding rule and
      a floor of 1; `[vm]` carries no CPU field.
- [ ] Memory size is derived from `resources.memory_bytes`; `[vm]` carries no memory field.
- [ ] Both are observable **from inside the guest** in a Tier-3 case — not asserted against
      the hypervisor config the platform generated.
- [ ] `workload describe` reports the declared resources for a VM allocation as it does for
      a process allocation.
- [ ] **Sizing holds on both memory backings.** The Tier-3 sizing case is parametrized over
      a VM with **no** volume (private memory) and a VM **with** a volume
      (`--memory shared=on`, per `[D8b]`), because `shared=on` is the one thing in this
      feature that changes how guest memory is backed. This closes the
      `shared=on` × sizing interaction **inside** the feature rather than leaving GH #92 to
      discover it. *(Cost: one parametrized case, not a second test — which is why Slice 04
      is ordered before Slice 05.)*

#### Technical Notes

- This is the #92 precondition: the right-sizing reconciler resizes against **one**
  declared size via `Driver::resize`.
- `Driver::resize` is a required trait method; whether Slice 05 implements live hotplug or
  rejects resize honestly is DESIGN's call — but it must not silently no-op.
- **`[D8e]` interaction, stated:** a volume-carrying VM's `virtiofsd` sits in the
  allocation's cgroup scope, so its memory counts against `resources.memory_bytes`. DESIGN
  decides whether the guest's memory size is derived from the declared figure as-is or net
  of a daemon allowance — but it must **not** be left implicit, or a volume-carrying VM
  silently gets less guest RAM than an identical volume-free one.

---

### US-VM-6 — A `[vm]` + `[service]` spec is rejected with an honest error naming what is missing

**Job:** `J-OPS-003` · **Slice:** 02 · **MoSCoW:** Must

#### Elevator Pitch

- **Before:** Nothing stops Ana writing `[vm]` + `[service]`. If the platform accepts it,
  she gets a workload that is scheduled, reports `Running`, and is **unreachable** — no
  tap in its netns, no mesh identity, no probe that can see inside the guest. She
  discovers this by debugging a service that silently receives no traffic.
- **After:** `overdrive deploy web.toml` with `[vm]` + `[service]` is **rejected at
  deploy time** with an error naming the missing capabilities (guest networking, guest
  probes, guest-stack mTLS interception) and pointing at GH
  [#257](https://github.com/overdrive-sh/overdrive/issues/257) (tap-in-netns +
  guest-reachable probes) and
  [#222](https://github.com/overdrive-sh/overdrive/issues/222) (guest-stack mTLS
  intercept).
- **Decision enabled:** Ana learns in one second, at deploy time, that VM services are not
  yet supported — instead of losing an afternoon to a service that deploys cleanly and
  serves nothing.

#### Problem

`vision.md` principle 3 says security is structural, not configurable. A VM workload
cannot currently be mesh-enrolled (`[D6]`): the interception layer is structurally blind
to a guest's TCP stack, and probes cannot reach inside a guest. Accepting a spec the
platform cannot honour is the failure mode this whole feature exists to avoid.

#### Who

Ana Moreno | reaching for the workload kind she uses most, on a driver that cannot yet
serve it.

#### Solution

Reject `[vm]` + `[service]` at parse/validation time with an error that names each
missing capability and cites the tracking issue. `[vm]` + `[job]` and `[vm]` +
`[schedule]` are accepted — a scheduled VM job is the same mechanism on a timer and needs
no new machinery.

#### Domain Examples

##### 1. Rejected at deploy time

Ana writes `[service]` + `[vm]` for `web`. `overdrive deploy web.toml` fails immediately,
naming guest networking, guest probes and guest-stack mTLS, and citing #257 and #222.
Nothing is scheduled.

##### 2. Job kind accepted

The same `[vm]` block under `[job]` deploys and runs.

##### 3. Schedule kind accepted

`[schedule]` + `[vm]` with a cron expression deploys; the VM job runs on its schedule.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A VM service spec is rejected with the reason it cannot be served
  Given Ana has written a spec declaring both [service] and [vm]
  When she runs "overdrive deploy web.toml"
  Then the deploy is rejected before anything is scheduled
  And the error names guest networking, guest probes and guest-stack mTLS as missing
  And the error cites a tracking issue for each named missing capability

Scenario: A VM job spec is accepted
  Given Ana has written a spec declaring both [job] and [vm]
  When she runs "overdrive deploy render.toml"
  Then the workload is accepted and scheduled

Scenario: A scheduled VM job is accepted
  Given Ana has written a spec declaring [schedule] with a cron expression and [vm]
  When she runs "overdrive deploy nightly.toml"
  Then the workload is accepted and scheduled
```

#### Acceptance Criteria

- [ ] `[vm]` + `[service]` is rejected at deploy time; **no allocation is created and no
      intent is committed.**
- [ ] The rejection message names each missing capability (guest networking, guest-reachable
      probes, guest-stack mTLS interception) and cites a real issue for each: **GH #257**
      (tap-in-netns provisioning + guest-reachable probes) and **GH #222** (guest-stack mTLS
      intercept). No named gap resolves to a hand-wavy forward pointer.
- [ ] `[vm]` + `[job]` and `[vm]` + `[schedule]` are accepted and run.
- [ ] The rejection is produced by a real `overdrive deploy`, observable in CLI output.

#### Technical Notes

- Mirrors the existing `ParseError::ProbesNotAllowedOnKind` precedent
  (`workload_spec.rs:827`) — a semantic rejection with guidance, not a parse error.
- This rejection is **removed**, not relaxed, when VM services become supported.

---

### US-VM-7 — The hypervisor process is confined, or the workload does not run

**Job:** `J-OPS-003` · **Slice:** 03 · **MoSCoW:** Must

#### Elevator Pitch

- **Before:** *(counterfactual — there is no VM path in the tree today, `[G1]`.)* Absent this
  story, the hypervisor Slice 01 spawns would run as **root**, with the host's default
  file-size and descriptor limits, and read/write reach over the entire host filesystem — a
  guest that escaped into the VMM would inherit all of it. Ana would have no way to know,
  and the platform's own docs would happily imply otherwise: the reference implementation
  asserted *"VM isolation identical to Firecracker"* while its research recorded the opposite.
- **After:** Ana runs `overdrive deploy untrusted-render.toml` on a host that cannot supply
  the required confinement → `overdrive workload describe untrusted-render` shows
  **`Failed`**, with a reason naming the confinement that was unavailable. On a host that
  can, the deploy is indistinguishable from any other VM job — and the guarantee holding
  behind it is a hypervisor running as a non-root uid, under a Landlock ruleset scoped to
  that VM's own kernel / rootfs copy / API socket, with bounded `fsize` and `no-file`.
  **The confinement itself is never rendered to Ana — only its absence is**, which is
  precisely what makes the `[D7]` claim checkable rather than narrated.
- **Decision enabled:** Ana can decide whether to run an artifact she does not fully trust —
  because the posture is either exactly the one `[D7]` documents, or the workload does not
  start at all. She never has to guess which, and she never has to inspect a process to
  find out.

#### Problem

`vision.md` principle 3 is *"security is structural, not configurable."* Cloud Hypervisor
ships no jailer, so without deliberate work the hypervisor is an ordinary root process with
the whole host in reach. Precedent warning #6 is what happens when that gap is papered over
in prose instead of closed in code: the claim shipped, the hardening did not, and the
refuting evidence sat one file away in the same repository.

#### Who

Ana Moreno | running a VM precisely *because* she wants a stronger boundary than a host
process | no hypervisor-hardening expertise assumed | reads the platform's isolation claim
as a promise, exactly as she reads `Running`.

#### Domain Examples

##### 1. Happy path — an untrusted render runs bounded

Ana deploys `untrusted-render.toml` on the appliance (pinned 6.18 kernel). The VM boots.
For that allocation's hypervisor PID — resolved through the allocation's cgroup
`cgroup.procs` — `/proc/<pid>/status` shows a non-zero `Uid:` and `Seccomp:` mode, and
`/proc/<pid>/limits` shows `Max file size` and `Max open files` strictly below the control
plane's own. It was launched under a ruleset naming only that allocation's kernel, rootfs
copy and API socket — the same ruleset shape Slice 00 P5 proved denies an open of
`/etc/shadow`.

##### 2. Boundary — an artifact outside the default directory still boots

Ana's rootfs is at `/home/ana/scratch/render.ext4`, not
`/var/lib/overdrive/artifacts/`. The VM boots: the Landlock ruleset is derived from **the
paths this spec actually declares**, not from a hardcoded artifact directory.

##### 3. Error — a host that cannot confine refuses the workload

Ana deploys on a dev host whose kernel does not offer the Landlock support the platform
requires. `overdrive workload describe` reports `Failed`, naming the unavailable
confinement. The VM never boots — it does not fall back to running unconfined.

#### UAT Scenarios (BDD)

```gherkin
Scenario: An untrusted VM workload runs with a bounded hypervisor
  Given Ana has deployed a VM workload on a host that supports the required confinement
  When the allocation reaches Running
  Then the hypervisor process for that allocation runs as a non-root user
  And its resource limits are strictly tighter than the control plane's own
  And the ruleset confining it names only that allocation's own artifacts

Scenario: The confinement ruleset follows the operator's declared artifact paths
  Given Ana has deployed a VM workload whose rootfs lives outside the default artifact directory
  When the allocation starts
  Then the VM boots successfully
  And the hypervisor can reach the declared kernel and rootfs and nothing else

Scenario: A host that cannot confine the hypervisor refuses the workload
  Given Ana has deployed a VM workload on a host that cannot supply the required confinement
  When the platform attempts to start the allocation
  Then the allocation is Failed with a reason naming the unavailable confinement
  And the hypervisor is never started unconfined

Scenario: Confinement does not change how a VM workload is deployed or read
  Given Ana already deploys VM jobs with "overdrive deploy <spec>"
  When she deploys a workload on a host that supports the required confinement
  Then no new flag, table or verb is required
  And the terminal state and exit code she reads are unchanged
```

#### Acceptance Criteria

Every criterion is observed against the **live hypervisor process of a real allocation**
started by `overdrive serve` + `overdrive deploy` — never by asserting that the driver
constructed a flag.

- [ ] **Item 2 — uid/gid drop.** `/proc/<vmm-pid>/status` reports a non-zero real *and*
      effective `Uid:` and `Gid:` for the running hypervisor.
- [ ] **Item 3 — bounded rlimits.** `/proc/<vmm-pid>/limits` reports a finite `Max file
      size` and `Max open files`, **both strictly lower than the same fields on the
      `overdrive serve` process** — `/proc/<serve-pid>/limits`, and `/proc/self/limits` only
      where the harness *is* that process. *(Named by PID rather than `self` because under a
      Tier-3 harness `self` is the test process, whose limits need not match the server's —
      an ambiguous anchor would make the comparison unfalsifiable.)* The strictly-lower
      comparison is the binding half; it cannot be satisfied by inheriting the host default.
      DESIGN sets the absolute ceiling; the comparison holds whatever it picks.
- [ ] **Item 1 — Landlock enforced, and scoped to this VM.** Two halves, each with a named
      executor:
      **(a) positive, on the production path** — the allocation boots, which proves the
      hypervisor reached its declared kernel, rootfs copy and API socket *under* the ruleset;
      and **Domain Example 2 boots**, proving the ruleset is derived from the spec's declared
      paths rather than a hardcoded artifact directory. This is the falsifiable production
      assertion: a hardcoded ruleset fails Example 2.
      **(b) denial evidence — inherited from Slice 00 P5**, where the probe controls the
      process, applies the identical ruleset, and attempts an open outside it. *A live
      `cloud-hypervisor` cannot be induced to open an arbitrary path — it exposes no such
      command — and a sibling test process is not covered by the VMM's ruleset, so neither
      is available on the production path. The remaining runtime proof (that Landlock is
      active on a **running** VM) is explicitly **#258's runtime-EDD item** and is not
      claimed here.* **Argv presence is not acceptable evidence for either half.**
- [ ] **Fail-closed, with a named producer.** On a host that cannot supply the confinement,
      the allocation reaches `Failed` with a distinct reason naming what was unavailable —
      **a fifth variant minted in the Slice 02 shape**, not a reuse of one of its four
      (US-VM-2's "no two share a variant" holds; K3 targets "≥ 4 distinct", so the
      vocabulary extends rather than collides). **The unavailable-confinement condition is
      injected at the `Vmm` port boundary** — permitted by system constraint 1 (*"a `Sim*`
      adapter injected at a port boundary is fine"*), and necessary because the whole test
      envelope runs on one Lima kernel, so no genuinely Landlock-less host exists in it.
      DESIGN pins the seam; DISCUSS fixes only that the seam is at the port and **not** a
      test that hand-supplies a production effect.
      **No code path starts the hypervisor with confinement silently degraded** — mutation
      target: a mutation turning the fail-closed arm into warn-and-continue **must be
      killed**.
- [ ] **No new operator surface.** No flag, no `[vm]` key, no verb. Confinement is
      structural, per `vision.md` principle 3.
- [ ] **The claim matches the code.** No artifact or string asserts isolation beyond `[D7]`;
      the forbidden sentence *"isolation identical to Firecracker"* appears nowhere.

#### Technical Notes

- Depends on **Slice 00 P5** having proven these flags compose with a real CH boot, and on
  **Slice 01** having built the spawn path they attach to.
- **Open for DESIGN — which uid.** A uid-dropped `cloud-hypervisor` still needs `/dev/kvm`
  (typically group `kvm`), its API-socket directory, and the per-launch rootfs copy.
  **Constraint: the identity must be resolvable without appliance-image changes inside this
  feature** (e.g. an existing unprivileged uid plus group membership). If DESIGN finds it
  cannot be, that is a surfaced blocker for the user — not an assumption to design past, and
  not a silent escalation into ADR-0068 appliance territory.
- Item 5 (cgroup + netns placement) and item 6 (seccomp not weakened) are **not here** —
  they are inherent to `VmDriver::start` and land in US-VM-1. Item 4 (mount namespace) is
  **not in this feature** — GH #258, see `[D7]`.
- Tests run as root under Lima (`.claude/rules/testing.md`), so a uid-drop assertion is only
  meaningful if the test observes the *spawned* process's identity rather than its own.

---

### US-VM-8 — A VM job writes its output to a host directory the operator can read

**Job:** `J-OPS-003` · **Slice:** 04 · **MoSCoW:** Must

#### Elevator Pitch

- **Before:** Ana's `batch-render` VM boots, runs, and exits `0` — and produces nothing she
  can reach. Its rootfs is a per-launch reflink copy (`[D5]`) discarded at terminal *by
  design*, so every byte the guest wrote is thrown away. The only thing that survives the
  VM is an integer.
- **After:** Ana adds three lines —
  `[[vm.volume]]` with `source = "/var/lib/overdrive/outputs/batch-render"` and
  `target = "/output"` — and runs `overdrive deploy render.toml`. The render writes
  `/output/frame-0120.exr` inside the guest; when the job terminates, `ls
  /var/lib/overdrive/outputs/batch-render` **on the host** lists `frame-0120.exr`.
- **Decision enabled:** Ana can decide whether to put real batch work on the VM class at
  all. A workload whose output she cannot retrieve is a demo; one whose output lands in a
  directory she names is a deliverable she can pipe into the next step.

#### Problem

The `[vm]` + `[job]` workloads this feature exists to run — a render, a checksum, a
transcode — are valuable *because of the artifact they produce*, not because of their exit
code. `[D5]`'s per-launch reflink copy is deliberately thrown away at terminal (it is what
makes restart honest), so without a volume there is **no** path for a guest's bytes to
reach the host. The platform would ship a workload class that can compute but cannot
deliver.

#### Who

Ana Moreno, Overdrive platform engineer | running batch VM jobs whose entire value is the
file they emit | reads the output directory with ordinary host tools (`ls`, `cat`, a
downstream script), not through Overdrive.

#### Solution

A `[[vm.volume]]` array in `[vm]` naming a host `source`, a guest `target`, and an optional
`read_only` flag (`[D8a]`). Per volume-carrying VM the platform supervises one `virtiofsd`
over a vhost-user socket, derives `--memory shared=on` from volume presence (`[D8b]`), runs
`--cache=never` (`[D8c]`), and mounts the share in the guest at `target`. A spec with no
volume is byte-identically Slice 01's VM.

#### Domain Examples

##### 1. Happy path — the render's frames survive the VM

Ana declares `source = "/var/lib/overdrive/outputs/batch-render"`, `target = "/output"` and
runs `/usr/bin/render --frames 120 --out /output`. The job reaches
`Terminated / Completed{exit_code: 0}`. On the host,
`/var/lib/overdrive/outputs/batch-render/frame-0120.exr` exists and is readable.

##### 2. Read-only input — a shared dataset the guest cannot corrupt

Ana declares a second volume, `source = "/srv/datasets/hdri"`, `target = "/data"`,
`read_only = true`. The guest reads `/data/studio.hdr` successfully; a guest write to
`/data/scratch` fails inside the guest, and `/srv/datasets/hdri` on the host is
byte-unchanged.

##### 3. Error — the volume source directory does not exist

Ana typos `outputs/batch-rendr`. The allocation reaches `Failed` with a reason naming **the
volume source directory and the path that was not found** — distinct from a missing rootfs,
a missing kernel, and a missing storage daemon. No VM is booted.

##### 4. Error — a BYO kernel without virtiofs support fails loudly, not silently

Ana points `[vm] kernel` at her own `vmlinux-6.12-custom`, built without
`CONFIG_VIRTIO_FS`. The VM boots and `overdrive-init` cannot mount `/output`, so **it
refuses to exec `/usr/bin/render`**. The allocation reaches `Failed` naming the volume that
could not be mounted. Ana's render **never runs** — which is the point: had it run, it would
have written 120 frames into the per-launch rootfs copy, exited `0`, and been reported as a
successful job over an empty output directory.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A guest's write to a declared volume is readable on the host
  Given Ana has deployed a VM job declaring a volume from a host directory to a guest path
  And her guest command writes a file to that guest path
  When the job reaches a terminal state
  Then the file is present and readable in the host directory she named
  And its contents are byte-identical to what the guest wrote

Scenario: A read-only volume refuses guest writes and leaves the host untouched
  Given Ana has deployed a VM job declaring a read-only volume
  When the guest attempts to write inside that volume
  Then the write fails inside the guest
  And the host directory is byte-unchanged

Scenario: A VM job that declares no volume behaves exactly as before
  Given Ana has deployed a VM job with no volume declared
  When the guest runs and exits
  Then the allocation reaches the same terminal state and exit code as it did before volumes existed
  And no storage daemon is started for that allocation

Scenario: A missing volume source directory is named precisely
  Given Ana has deployed a VM job whose declared volume source directory does not exist
  When the platform attempts to start the allocation
  Then the allocation is Failed with a reason naming the volume source and the path
  And the reason is distinct from a missing rootfs and from a missing storage daemon

Scenario: A missing storage daemon is distinguished from a missing directory
  Given the host has no virtiofs storage daemon installed
  When Ana deploys a VM job declaring a volume whose source directory exists
  Then the allocation is Failed with a reason naming the missing storage daemon

Scenario: A volume that cannot be mounted in the guest never reports a completed run
  Given Ana has deployed a VM job declaring a volume
  And the guest cannot mount that volume
  When the platform starts the allocation
  Then the allocation is Failed with a reason naming the volume that could not be mounted
  And her command is never run
  And the allocation never reaches a completed terminal state

Scenario: Adding a volume does not widen what the hypervisor can reach
  Given Ana has deployed a VM job declaring a volume
  When the allocation reaches Running
  Then the hypervisor's reach is unchanged from a VM job that declares no volume
  And the hypervisor cannot reach the volume's host directory
```

#### Acceptance Criteria

Derived from the seven scenarios above. Engineering constraints that are **not** UAT-derived
live in their own block below.

- [ ] *(Scenario 1)* A guest write inside a declared volume is present, readable and
      byte-identical in the operator's host `source` directory after the job terminates —
      proven by a Tier-3 case through a real `overdrive serve` + `overdrive deploy`, with
      the host-side read done by ordinary filesystem access, **not** through any Overdrive
      API.
- [ ] *(Scenario 2)* `read_only = true` is enforced **host-side** (`[D8g]`): the guest write
      fails and the host directory is byte-unchanged. **Observed from inside the guest and
      on the host** — not asserted against the flag the platform passed. **The Tier-3 case
      must defeat a guest-side-only implementation**: it is not sufficient for a cooperative
      guest's write to fail, because a guest-side `-o ro` can be remounted away by an
      uncooperative one and would leave the security framing in `[D8a]` unearned.
- [ ] *(Scenario 3)* A spec declaring **no** volume starts **no** storage daemon and derives
      `--memory shared=on` **off** (`[D8b]`) — its terminal state and exit code are
      unchanged from Slice 01. This is the regression guard on slices 01–03.
- [ ] *(Scenarios 4, 5)* Volume-source-not-found and storage-daemon-absent are **two
      distinct** `TransitionReason` variants, minted in the Slice 02 shape; neither collides
      with that slice's four nor with US-VM-7's fifth. Each names the resource and the
      actionable next step, verified by reading `workload describe` output rather than by
      asserting on an enum.
- [ ] *(Scenario 6)* **A failed required mount is `Failed`, never a completed run.** A
      volume the guest cannot mount yields its own named `TransitionReason` (a third
      variant), the operator's command is **never executed**, and the allocation **never
      reaches a completed terminal state**. Proven by a Tier-3 case in which the mount is
      made to fail while everything else succeeds — **the composite-lie case**: without this,
      the command writes into the discarded per-launch rootfs copy at `target`, exits 0, and
      `workload describe` reports `Terminated / Completed{exit_code: 0}` over an **empty**
      host directory, with every individual signal truthful (`[D4]` amendment, `[D8g]`).
- [ ] *(Scenario 7)* **`[D8e]` — volumes do not widen the hypervisor's confinement.** The
      volume `source` directory does **not** appear in the live `cloud-hypervisor` process's
      Landlock ruleset; only the storage daemon reaches the data. Verified against the live
      allocation, not against the constructed ruleset. **And the rest of the `[D7]` posture
      is unchanged for a volume-carrying allocation** — `/proc/<vmm-pid>/{status,limits,cgroup,ns/net}`
      match US-VM-1 item 5 and US-VM-7 items 2–3 on an allocation the Slice 04 Tier-3 case is
      already booting, so K7 covers volume-carrying VMs and not only volume-free ones.

#### Engineering Constraints (binding; Definition of Done, not UAT-derived)

- [ ] **The operator surface is closed.** The only keys `overdrive deploy` accepts under
      `[[vm.volume]]` are `source`, `target` and `read_only`; any other key is **rejected by
      name** (`[D8a]`). Stated as the observable rather than as "`--cache` is
      platform-derived", because a construction property over code that never names the flag
      has no test and no mutation site.

#### Technical Notes

- Depends on **Slice 00 P6** (virtiofsd + `--memory shared=on` compose with a real boot
  *and* with the `[D7]` confinement flags) and on **Slices 01–03**.
- `[[vm.volume]]` is an additive `[vm]` field. Whether it reaches the persisted `Job`
  aggregate — and therefore whether it rides inside Slice 01's `JobEnvelope` **V2** or needs
  its own bump — is a **DESIGN** question; `[G4]`'s six-step procedure governs either way,
  and the answer must be settled before Slice 04 is built, not during it.
- The `virtiofsd` process is placed in the allocation's cgroup scope and netns (`[D8e]`);
  its memory therefore counts against the workload's declared limit, which interacts with
  US-VM-5 and is why Slice 04 precedes Slice 05.
- **The guest-side mount is `overdrive-init`'s job** — the `[D4]` amendment and `[D8g]`.
  This *expands* `[D4]`'s locked agent scope from four duties to five, which is recorded as
  an explicit amendment rather than inferred from a passive sentence. It also extends the
  BYO-artifact contract: the operator's kernel must provide virtiofs support.
- **`read_only` is enforced host-side** (`[D8g]`). A guest-side `-o ro` is applied too, but
  as an ergonomic guard only — it is guest-cooperative and is not the boundary.
- Ordering, supervision, and the daemon's death are **US-VM-9**, not here. This story is the
  capability; that one is its honesty.

---

### US-VM-9 — A storage daemon's death is classified as honestly as the hypervisor's

**Job:** `J-OPS-003` · **Slice:** 04 · **MoSCoW:** Must

#### Elevator Pitch

- **Before:** *(counterfactual — there is no volume path in the tree today.)* The reference
  implementation treated **any** `virtiofsd` exit — **including a clean one at teardown** —
  as `VmmError::Crash("virtiofsd crashed unexpectedly")` and force-killed the VM, so a job
  that finished successfully could be reported as crashed **by its own shutdown sequence**.
  And nothing made a `virtiofsd` that died *mid-run* reach `ExitKind` at all: the guest's
  writes start failing, the workload ends however it ends, and the platform reports that
  ending as though storage had been fine the whole time.
- **After:** Ana's `batch-render` finishes and shuts down → `overdrive workload describe
  batch-render` shows `Terminated / Completed{exit_code: 0}`, and the storage daemon's own
  clean exit appears nowhere in that verdict. When the storage daemon dies **mid-run**
  instead, the same command shows **`Failed`** with a reason naming the storage daemon —
  never a clean exit, and never a silently truncated output.
- **Decision enabled:** Ana can trust a green terminal state on a volume-carrying VM exactly
  as much as on one without. She never has to ask "did the output actually get written, or
  did the share die halfway and the job report success anyway?"

#### Problem

`[D3]` and system constraint 9: **a supervised sidecar is not the workload, and its exit
status is not evidence about the workload's.** Volumes add the first such sidecar, and the
reference implementation got the classification wrong in *both* directions at once — clean
sidecar exit read as a crash, mid-run sidecar death not read at all. Either direction
produces exactly the class of lie US-VM-3 exists to prevent, one process over. A truncated
output file that is reported as a successful run is worse than a failed run, because Ana
builds on it.

#### Who

Ana Moreno | relying on a volume-carrying VM's terminal state to gate downstream work that
consumes the output file | has no way to distinguish "the job wrote 120 frames" from "the
job wrote 40 frames and the share died" except by trusting the platform.

#### Solution

Classify by the **workload's** reported outcome, never by the daemon's exit status
(constraint 9). Order the lifecycle so the share is ready before the guest boots and torn
down after the VM, with no leaked socket or orphan on any failure path. Launch the daemon
`--sandbox=namespace` and **fail closed** rather than silently downgrade (`[D8d]`).

#### Domain Examples

##### 1. Clean teardown — the daemon's own exit is not a crash

`batch-render`'s guest command exits `0`, the agent reports it, the VM shuts down, and the
storage daemon exits cleanly as part of that teardown. The allocation is
`Terminated / Completed{exit_code: 0}`. The daemon's exit contributes nothing to the
verdict.

##### 2. Mid-run daemon death — reported as failed, naming the storage daemon

`batch-render` is 40 frames into 120 when its storage daemon is killed. The guest's writes
begin failing. `workload describe` reports `Failed` with a reason naming the storage
daemon's death — not `Completed`, and not a bare hypervisor error string. Backoff retries
the allocation.

##### 3. Error — a host that cannot sandbox the daemon refuses the workload

Ana deploys on a dev host where `--sandbox=namespace` is unavailable.
`workload describe` reports `Failed` naming the unavailable storage sandbox. **The daemon
is never started under `chroot` instead** — which is precisely what the reference
implementation silently did.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A completed VM job is not reported as crashed by its own shutdown
  Given Ana has deployed a VM job declaring a volume whose guest command exits 0
  When the job completes and the platform tears the allocation down
  Then the allocation is Terminated with completed exit code 0
  And the storage daemon's own exit does not change that verdict

Scenario: A storage daemon that dies mid-run never produces a clean exit
  Given Ana has deployed a running VM job declaring a volume
  When the storage daemon dies while the guest is still running
  Then the allocation is Failed with a reason naming the storage daemon
  And the allocation is not Terminated with a completed condition

Scenario: The guest does not boot before its share is ready
  Given Ana has deployed a VM job declaring a volume
  When the platform starts the allocation
  Then the guest does not begin booting until the share is ready to serve
  And if the share never becomes ready the allocation is Failed naming that wait

Scenario: A host that cannot sandbox the storage daemon refuses the workload
  Given Ana has deployed a VM job declaring a volume on a host that cannot supply the required storage sandbox
  When the platform attempts to start the allocation
  Then the allocation is Failed with a reason naming the unavailable sandbox
  And the storage daemon is never started with a weaker sandbox instead

Scenario: Nothing is left behind after a volume-carrying VM ends
  Given Ana has deployed a VM job declaring a volume
  When the allocation reaches any terminal state, including a failed start
  Then no storage daemon process remains for that allocation
  And no vhost-user socket remains on the host
```

#### Acceptance Criteria

> **ACs 1–3 are ONE discriminated classification, not three independent checks — and they
> must not be implemented as three.** The discriminator is a single guard: *did the daemon's
> exit arrive **before** the workload reported an outcome, or during teardown?* Naming it
> here because AC 1 alone is **vacuous without it**: a do-nothing implementation that simply
> ignores the daemon also produces `Terminated / Completed`, and cargo-mutants cannot
> *insert* a guard that the code does not contain. AC 1 is non-vacuous only because AC 2's
> daemon watcher forces that guard into existence. This is the same trap already closed
> twice in this document — US-VM-1 item 6 (an argv assertion, because a "no code path
> constructs X" criterion over code that never mentions X has no mutation site) and US-VM-7
> item 1 (two halves, each with a **named executor**).

- [ ] *(Scenario 1)* **A clean storage-daemon exit is never a crash.** A VM job that
      completes normally reaches `Terminated / Completed{exit_code: N}` with the daemon's
      own exit contributing nothing. **The Tier-3 case must discriminate, not merely
      observe a green result: assert that the daemon's exit WAS observed** (it appears in
      the allocation's event/audit trail) **while contributing nothing to `ExitKind`** — an
      observation a do-nothing implementation cannot produce. **Mutation target: removing or
      negating the before-vs-during-teardown guard must be killed by this case** — that
      guard is the site, and it is the reference implementation's defect reproduced as a
      test obligation.
- [ ] *(Scenario 2)* **A mid-run storage-daemon death is never a clean exit.** It yields
      `Failed` with a distinct reason naming the storage daemon, proven by a Tier-3 case
      that kills the daemon while the guest is running. **Mutation target: a mutation
      collapsing that arm into `CleanExit` must be killed.** This is the arm that makes the
      guard exist.
- [ ] *(Scenarios 1, 2)* Classification uses the **workload's** reported outcome as the
      discriminator, per constraint 9. **No code path derives `ExitKind` from the storage
      daemon's exit status** — in either direction.
- [ ] *(Scenario 3)* **Ordering is enforced, not assumed.** The daemon is running **and its
      socket is ready** before `vm.create`; the VM is torn down before the daemon. A
      readiness wait that expires is its own named `Failed` reason, distinct from
      volume-source-missing and storage-daemon-absent (US-VM-8).
- [ ] *(Scenario 4)* **`--sandbox=namespace` or nothing** (`[D8d]`). A host that cannot
      supply it lands `Failed` with a named reason. **No code path starts the daemon with a
      weaker sandbox** — mutation target: turning the fail-closed arm into
      downgrade-and-continue **must be killed**. As with US-VM-7, the unavailable condition
      is injected at the port boundary (system constraint 1), since the whole test envelope
      runs on one Lima kernel.
- [ ] *(Scenario 5)* **No leak on any path.** After any terminal state — including a failed
      start, a readiness timeout, and a sandbox refusal — no daemon process and no
      vhost-user socket remain. Verified for the allocation's cgroup scope (`[D8e]` places
      the daemon there precisely so `cgroup_kill` reaps it) and for the socket path on disk.
- [ ] Every case is driven through a real `overdrive serve` + `overdrive deploy`, never by
      a test-invoked driver method.

#### Technical Notes

- Extends `[D3]`'s classification with a second supervised process; it does **not** mint a
  parallel model. `exit_observer::classify` and `workload_lifecycle.rs:673-743` are the same
  consumers as US-VM-3.
- The daemon's death, the VMM's exit, and the agent's report can all race. **DESIGN pins the
  ordering** so a workload outcome that has already been reported is never overwritten by a
  subsequent teardown — the same pin US-VM-3's technical note asks for, now with three
  participants instead of two.
- `--sandbox` mode *selection* and its fail-closed rule are this story's (a spawn-argument
  property of the launch this feature owns). The daemon's deeper **posture** — its seccomp
  set, its uid, xattr surface, its own threat model — is
  [#258](https://github.com/overdrive-sh/overdrive/issues/258), **unconditionally**. See
  `[D7]`'s boundary table.
- Depends on **US-VM-8** (same slice, must land together), **Slice 02** (the failure
  vocabulary this story extends), **Slice 03** (`[D3]`'s classification and the bounded-grace
  shutdown shape it reuses), and **Slice 00 P6**.

---

## `[REF]` Outcome KPIs

### Objective

Ana can run a workload that needs its own kernel through the same verbs she already
trusts, and the platform never tells her something about a VM that is not true.

### KPI table

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | VM allocations that reach a terminal state | report the **guest's** exit code, not the hypervisor's | **100%** — zero divergence | n/a (no VM path exists) | Tier-3 matrix over guest exit codes {0, 1, 7, kernel-panic, VMM-killed}; assert `workload describe` matches the guest's real status | Leading |
| K2 | VM allocations whose guest never ran init | reach `Running` | **0** | n/a | Tier-3 case: rootfs with absent/broken init must land `Failed` without passing through `Running` | Leading (guardrail) |
| K3 | Distinct VM start-failure modes | surface a distinct, named operator reason rather than a raw string | **≥ 4 distinct** (kernel missing, rootfs missing, hypervisor absent, boot timeout); **0** collapsed into `DriverInternalError` | 0 of 4 — all collapse today (`[G6]`) | Read `workload describe` output per Tier-3 failure case | Leading |
| K4 | The production composition path | can reach the VM driver via `overdrive serve` + `overdrive deploy` | **binary: yes** | **no** — `lib.rs:1422` hardcodes `ExecDriver` | A real `serve` + `deploy` boots a VM with **no** test-only wiring | Leading (the feature's pass/fail bar) |
| K5 | A crashed VM allocation | is restarted by backoff on the same terms as a crashed process | **parity** — same ceiling, same backoff, 0 divergence | n/a | Tier-3 crash-restart case compared against the exec baseline | Leading |
| K6 | Ana deploying a VM job | sees `Running` within a bounded time from `deploy` | p50 **≤ 3 s**, p99 **≤ 10 s**, single node, warm artifacts | n/a | Tier-3 timing over ≥ 20 deploys | Leading (secondary) |
| K7 | Running VM allocations **on an mTLS-composed production boot**, **volume-carrying and volume-free alike** | have a hypervisor process that is non-root, seccomp-filtered, Landlock-confined, cgroup-placed and netns-placed | **100%** confined; **0** started with confinement silently degraded; **0** whose confinement is widened by declaring a volume | n/a (no VM path exists) | Tier-3 read of `/proc/<vmm-pid>/{status,limits,cgroup,ns/net}` per allocation, PID resolved via the allocation's cgroup `cgroup.procs`; denial evidence inherited from Slice 00 P5; **the volume-carrying case is US-VM-8 AC 7, asserted on an allocation Slice 04 already boots** | Leading (guardrail) |
| K8 | VM allocations declaring a volume | have every guest write inside that volume present and byte-identical in the operator's host directory | **100%** — zero divergence, zero truncation | n/a (no volume path exists) | Tier-3 round-trip: guest writes a known payload, host reads it by ordinary filesystem access after terminal | Leading |
| K9 | VM allocations whose **storage daemon** died mid-run | are reported `Terminated / Completed` | **0** | n/a | Tier-3: kill `virtiofsd` while the guest runs; assert `Failed` naming the daemon. **Paired inverse, stated so it is not vacuous:** allocations completing normally are reported crashed **0** times **AND** the daemon's clean exit is *observed in the allocation's event trail while contributing nothing to `ExitKind`* — the bare "0 crashes" half is also achieved by a do-nothing implementation, so the observation is what makes it measure anything | Leading (guardrail) |
| K10 | VM allocations declaring `[resources]` | have guest-observed vCPU count and memory size matching the declared figure under the documented rounding rule | **100%**, on **both** memory backings | n/a (no VM path exists) | Tier-3 read from **inside the guest**, parametrized over private memory and `--memory shared=on` | Leading |

### Metric hierarchy

- **North star:** **K1** — exit-status fidelity. It is the one number that decides whether
  the feature shipped a capability or a lie, and it is exactly what the reference
  implementation could not satisfy. **K9 is K1's shape one process over** and inherits its
  standing for volume-carrying VMs.
- **Leading indicators:** K2 (Running honesty), K3 (diagnosability), K5 (lifecycle parity),
  **K8 (output fidelity — the outcome that makes the VM class *productive* rather than
  merely trustworthy)**, **K10 (declared-size fidelity — the #92 precondition)**.
- **Guardrails (must not degrade):** exec-driver behaviour is unchanged — all existing
  process-workload acceptance tests stay green; K2 must stay 0; **K7's degraded-start count
  must stay 0**; **K9 must stay 0 in both directions**; the DST suite stays green with
  `SimVmm` in the harness. **New with `[D8]`:** a VM declaring **no** volume must remain
  byte-for-byte the workload Slice 01 shipped — same memory backing, no storage daemon, same
  terminal states (US-VM-8 AC 3). Volumes must not regress the volume-free path.

### Measurement plan

| KPI | Data source | Collection | Frequency | Owner |
|---|---|---|---|---|
| K1, K2, K3, K5, K7, **K8, K9, K10** | Tier-3 integration matrix under Lima + the pinned 6.18 appliance kernel | `cargo xtask lima run -- cargo nextest run --features integration-tests` | Per PR | DELIVER |
| K4 | A real `overdrive serve` + `overdrive deploy` in the verification catalogue | `verification/harness/run-expectation.sh` | Per slice | DELIVER / DEVOPS |
| K6 | Timed Tier-3 deploys | Same harness, timing assertions | Per slice, and on the CH version floor changing | DELIVER |

### Hypothesis

We believe that a `Vmm`-backed `vm` driver with an in-guest agent, for a platform
engineer who needs kernel-level isolation, will let VM workloads be deployed and
trusted with the same verbs as process workloads. We will know this is true when
**every VM allocation's reported terminal state matches the guest's real outcome (K1 =
100%) and no allocation is ever reported `Running` for a guest that never started (K2 =
0)** — both measured on the production `serve` + `deploy` path.

---

## `[REF]` Risks

| Risk | P | I | Mitigation |
|---|---|---|---|
| **The mechanism composes but no production path reaches it** (precedent warning #1 — the reference implementation's exact failure) | Med | **Critical** | K4 is a named, binary KPI; `lib.rs:1422` is the first item in US-VM-1's **Engineering Constraints** block (binding Definition of Done — moved there from the AC list so it traces to a scenario-free constraint honestly, not weakened); slice briefs carry the "no test installs what production doesn't" taste test |
| **The pinned 6.18 kernel does not boot under CH from ext4/virtio-blk, or vsock misbehaves** | Med | High | **Slice 00 is a blocking spike** run for real under Lima on both kernels (precedent warning #2 is this risk realised) |
| **Exit honesty silently regresses to the VMM's exit code** | Med | **Critical** | US-VM-3 is a first-class story; the classification arm is a mandatory mutation-testing target |
| **The `Job` rkyv envelope bump is missed or split across commits** (`[G4]`) | Med | High | Called out as a Slice 01 AC; the 6-step procedure is single-commit by rule; existing golden fixtures are never touched |
| **Guest agent scope creeps toward `kata-agent`** | Med | Med | `[D4]` fixes the surface at beacon + exec + stdio + exit; #100 is the named growth path |
| **The isolation claim outruns the code** — precedent warning #6 realised (the reference implementation asserted *"isolation identical to Firecracker"* against its own research) | Med | **Critical** | `[D7]` is a locked, quoted claim; constraint 6 makes it the only sanctioned wording and names the forbidden sentence; US-VM-7 AC asserts no artifact exceeds it; the jailer remainder is #258 |
| **CH has no jailer; the hypervisor is an ordinary host process** (precedent warning #6) | High | Med | Partially closed in-feature by `[D7]` items 1–3, 5–6 (Landlock, uid/gid, rlimits, cgroup, netns, seccomp-not-weakened); the remainder — chroot, mount-ns, PID-ns, threat model — is **#258**, cited by number everywhere |
| **A uid-dropped `cloud-hypervisor` cannot open `/dev/kvm`, or the Landlock ruleset misses a path CH needs** — either fails opaquely *after* the spawn path is built | Med | High | **Slice 00 P5** proves the flags compose with a real boot before Slice 01 depends on them; US-VM-7 fails closed rather than degrading; the uid-identity question is an explicit DESIGN input with a no-appliance-changes constraint |
| **vsock cannot cross the per-workload netns**, breaking `[D2]`'s ready beacon once item 5 places the VMM | Med | High | Folded into **Slice 00 P2** — settled before Slice 01 builds on it, not discovered during it |
| **A CH version floor is asserted without a reason** (precedent warning #7) | Med | Low | Constraint 7: name what breaks below it, or declare none |
| **Scope creeps back toward Service-kind VMs** | Med | High | `[D6]` + US-VM-6 make the boundary an explicit, tested rejection rather than an omission |
| **A user input is dropped between intake and DISCUSS and never surfaced** — realised once already: `I-6` (virtiofsd) was stated at intake, lost, and silently reversed by research | **Realised** | High | `[D8]` records the recovery *and its mechanism* (rootfs was scoped, volumes never were); intake's own preamble rule — *"must not be silently reversed"* — is restated in `I-6`; **the structural fix is that DISCUSS now enumerates every intake decision `I-1..I-6` in its Governing-input block**, so a dropped one is visible as a gap rather than an absence |
| **The `virtiofsd` lifecycle is under-budgeted** — the reference's was 415 lines of socket-wait / signal-escalation / `Drop` guard | Med | Med | **Realised at DISCUSS and corrected** — the first `[D8f]` pass priced only the host-side lifecycle and came in at 4–6 d; peer review found five omitted rows (parse surface, `VmConfig` payload, guest-side mount, host-side `read_only`, a possible second rkyv bump) and it **re-budgeted to 6–9 d**. `[D8f]` now shows the line-by-line reused-vs-new split; **if DESIGN sizes it above 9 d, lifting Slice 04 into its own feature is pre-authorised** (§ Scope assessment guardrail 2, owner DESIGN, checkpoint the DESIGN handoff) |
| **A `virtiofsd` death is misclassified in either direction** — a clean exit read as a crash (the reference's exact defect) or a mid-run death read as a clean exit | Med | **Critical** | Constraint 9 states the rule; **US-VM-9 is a first-class story with mutation targets on both arms**; K9 measures both directions |
| **`--sandbox` silently downgrades from `namespace` to `chroot`** — precedent warning #6's shape, and exactly what the reference did with no rationale recorded | Med | High | `[D8d]` fixes `namespace` and makes downgrade **unrepresentable by failing closed**; US-VM-9 AC carries a mutation target on the fail-closed arm |
| **`--memory shared=on` costs more than assumed, or interacts with memory sizing** | Med | Med | `[D8b]` makes it **conditional** so volume-free VMs never pay it; **Slice 00 P6 measures the cost** rather than asserting it; Slice 04 precedes Slice 05 so US-VM-5's sizing case runs on both memory backings |
| **Volumes regress the volume-free path** that slices 01–03 already shipped | Low | High | `[D8b]`'s conditionality is the structural defence (one derived field on a `VmConfig` value); **US-VM-8 AC 3 is the explicit regression guard** — no volume ⇒ no daemon, no `shared=on`, unchanged terminal states |

---

## `[REF]` Definition of Ready

| DoR item | US-VM-1 | US-VM-2 | US-VM-3 | US-VM-4 | US-VM-5 | US-VM-6 | US-VM-7 | US-VM-8 | US-VM-9 |
|---|---|---|---|---|---|---|---|---|---|
| 1. Problem in domain language | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 2. Persona with specifics | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 3. 3+ domain examples, real data | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS (3) | PASS (3) |
| 4. UAT 3–7 scenarios | PASS (5) | PASS (4) | PASS (4) | PASS (3) | PASS (3) | PASS (3) | PASS (4) | PASS (7) | PASS (5) |
| 5. AC derived from UAT | **PASS w/ note** | PASS | PASS | PASS | PASS | PASS | PASS | **PASS w/ note** | PASS |
| 6. Right-sized | **PASS w/ note** | PASS | PASS | PASS | PASS | PASS | PASS | **PASS w/ note** | **PASS w/ note** |
| 7. Technical notes | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 8. Dependencies resolved or tracked | PASS | PASS | PASS | PASS | PASS | PASS | **PASS w/ note** | **PASS w/ note** | PASS |
| 9. Outcome KPIs | PASS (K1,K2,K4,K7) | PASS (K3) | PASS (K1,K5) | PASS (K5) | **PASS (K10)** | PASS (K3) | PASS (K7) | PASS (K8,K7) | PASS (K9) |

**DoR status: PASSED**, with seven recorded notes.

> **Two item-failures were found by peer review (2026-08-02, amendment 2) and FIXED, not
> waived.** **US-VM-8 item 5:** the story carried two ACs traceable to no scenario — the
> identical *technical-AC* defect review iteration 1 found in US-VM-1 and fixed there,
> reproduced in the new story. Closed by adding two UAT scenarios (a failed guest mount; the
> confinement non-widening) and moving the operator-surface criterion into an **Engineering
> Constraints** block, restated as an observable. **US-VM-5 item 9:** its cell was a bare
> `PASS` naming no KPI, and none of K1–K9 measured declared-resource fidelity — the story
> whose whole value is unblocking #92 had no measurement. Closed by minting **K10**.

**Item 5 note — US-VM-1's criteria were split, because six of them were not UAT-derived.**
Peer review (2026-08-02) found US-VM-1 carrying 10 criteria against 4 scenarios, with the
composition-root, `MicroVm`-deletion, rkyv-bump and `SimVmm` items untraceable to any
scenario — the *technical-AC* anti-pattern, and the fold had added two more. Fixed rather
than waived: a fifth scenario now covers hypervisor containment, the UAT-derived criteria
carry an explicit `*(Scenario N)*` back-reference, and the four engineering items moved to
a separate **Engineering Constraints** block. They are still binding — `lib.rs:1422` is the
feature's pass/fail bar — they are simply not acceptance criteria and were failing the item
by sitting in that list.

**Item 6 note — US-VM-1 is 5–8 days, above the 1–3 day norm.** This is the walking
skeleton. Three sub-splits were evaluated and each produces a *dead* increment rather
than a *thin* one (see § Scope assessment). The risk is mitigated by Slice 00 absorbing
the unknowns first, and by every other slice being genuinely thin. Recorded rather than
waived. **The `[D7]` fold did not change this number** — items 5 and 6 add ACs to work
already inside the slice, not mechanism.

**Item 8 note — US-VM-7 carries one unresolved DESIGN input: which uid the hypervisor
drops to.** It is *tracked, not open-ended*: the constraint is that the identity must be
resolvable without appliance-image changes inside this feature, and DESIGN returns a
blocker rather than improvising if it cannot be. Slice 00 P5 measures the `/dev/kvm`
access consequence before the story is built.

**Item 5 note — US-VM-8's criteria were split for the same reason US-VM-1's were.** Two of
its six original ACs (`[D8e]` non-widening; the operator-surface closure) traced to no
scenario. Fixed rather than waived: scenarios 6 and 7 were added — a failed guest mount, and
the confinement non-widening — bringing it to seven, and the surface criterion moved to an
**Engineering Constraints** block restated as an observable (*"any other key is rejected by
name"*) because *"`--cache` is platform-derived"* is a construction property over code that
never names the flag, and therefore has no test and no mutation site.

**Item 6 note — US-VM-8 (3.5–5 d) and US-VM-9 (2.5–4 d) both exceed the 1–3 day norm.**
This follows the `[D8f]` re-budget, which found the first pass had priced only the host-side
daemon lifecycle. They are recorded rather than waived, and they are **not** split further:
the two stories must land together (a slice shipping working volumes over a misclassified
daemon death is a slice whose deliverable is a lie), and splitting *within* either produces
a dead increment rather than a thin one — the same test applied to Slice 01's three
candidate sub-splits. The mitigation is the numbered lift trigger in § Scope assessment.

**Item 8 note — US-VM-8 carries one unresolved DESIGN input: whether `[[vm.volume]]`
reaches the persisted `Job` aggregate.** If it does, it rides inside Slice 01's
`JobEnvelope` **V2** (if Slice 04's shape is known by then) or needs its own bump; `[G4]`'s
six-step single-commit procedure governs either way, and the existing golden fixtures are
never touched. It is *tracked, not open-ended*: the procedure is fixed, only the placement
is open, and it must be settled **before** Slice 04 is built rather than discovered inside
it — which is the same mistake `[G4]` records the intake making for Slice 01.

**Traceability:** every story carries `job_id: J-OPS-003`. No `@infrastructure` stories —
each of the **nine** has an operator-invocable entry point in its Elevator Pitch "After"
line. Two are worth naming because their observable is indirect: **US-VM-7**'s is the
fail-closed `workload describe` reason (confinement is not rendered as operator output, but
its *absence* is), and **US-VM-9**'s is the terminal state itself — Ana never sees the
storage daemon, but she sees whether the verdict about her job is true of it. Both are the
observable that makes the corresponding claim checkable rather than narrated.

**Slice composition gate:** every **story-bearing** slice contains at least one plainly
operator-visible story. Slice 04's are **both** — US-VM-8 puts a file in Ana's directory,
US-VM-9 decides what `workload describe` says about the run that produced it. *(Slice 00
bears no stories at all: it is a throwaway spike governed by `.claude/rules/spike.md` that
produces no production code, so the gate does not apply to it rather than being satisfied
by it.)*

---

## `[REF]` Handoff to DESIGN

**Open for DESIGN (DISCUSS deliberately does not settle these):**

1. **Driver dispatch mechanism** — registry (ADR-0022's deferred pattern), enum dispatch,
   or a composite `Driver`. The *requirement* (`lib.rs:1422` changes; the shim routes by
   the spec's driver table) is fixed; the shape is not.
2. **The exact `Vmm` trait signature** — pinned in DESIGN per CLAUDE.md § "Implement to
   the design"; crafters must not improvise it. Constraint 4 fixes the *shape*
   (`VmConfig` value + one `create`), not the signature.
3. **The `From<JobV1> for JobV2` conversion body** — and *only* that. **Whether to bump is
   NOT open: the user ruled 2026-08-02 that `JobEnvelope` goes V1 → V2 via the full
   six-step single-commit procedure** (`[G4]`). Mutating V1 in place was considered and
   rejected because it requires regenerating a golden fixture, which
   `.claude/rules/testing.md` forbids outright. DESIGN owns the conversion, not the
   decision.
   **Sub-question:** whether `[[vm.volume]]` (`[D8a]`) reaches the persisted aggregate, and
   therefore whether it rides inside this same V2 or needs a later bump. Settle it before
   Slice 04 is built.
4. **The `AllocationSpec` driver-payload shape** — how VM artifact paths ride alongside
   the reused `command`/`args`.
5. **Agent ↔ host protocol over vsock** — beacon and exit framing; kernel cmdline vs vsock
   for delivering the entrypoint.
6. **vCPU derivation rule** from `cpu_milli`, and whether `Driver::resize` hotplugs or
   rejects honestly in **Slice 05**. Plus: whether guest memory is derived from
   `resources.memory_bytes` as-is or net of a `virtiofsd` allowance, given `[D8e]` places
   the daemon in the workload's cgroup scope.
7. **The `whitepaper.md:535` matrix correction** (CPU and memory hotplug collapsed into
   one row, marking Firecracker ❌ — wrong for memory since 2024-12-17) and ADR-0031:539's
   stale `[microvm]` table reference. **Route through the architect agent — do not edit
   inline.**

8. **Which uid/gid the hypervisor drops to** (US-VM-7 item 2), and how it retains `/dev/kvm`
   access. **Constraint: resolvable without appliance-image changes inside this feature.**
   If it is not, DESIGN returns a blocker rather than expanding into ADR-0068 territory.
9. **The Landlock ruleset's exact shape** (US-VM-7 item 1) — which paths, which access
   rights, and whether the CH version floor is declared on `--landlock` availability per
   constraint 7. Slice 00 P5 supplies the evidence. **Plus: the vhost-user socket path must
   be in the ruleset while the volume `source` directory must NOT be** (`[D8e]`) — the
   ruleset's *shape* is DESIGN's, but that exclusion is a DISCUSS decision and an AC.
10. **The `virtiofsd` ↔ VM ordering pin** (US-VM-9) — how "socket ready" is determined
    rather than slept on, and the race ordering where the daemon's death, the VMM's exit and
    the agent's report can all arrive together. Three participants, not two.
11. **The `VmConfig` volume payload shape** — how `[[vm.volume]]` entries, the derived
    `shared` flag, and the vhost-user socket path ride on the same `VmConfig` **value** that
    system constraint 4 requires. No stateful builder, no `attach_drive("output", …)`
    string-matched escape hatch (intake precedent warning #5 — the reference implementation
    intercepted the magic string `"output"` to mean "spawn a virtiofsd sidecar").

**Not open — DISCUSS decided:** `[D1]` `[vm]` contents; `[D2]` Running gate;
`[D3]` exit classification; `[D4]` agent yes/minimal; `[D5]` ext4 + virtio-blk **for the
rootfs**; `[D6]` Job-kind only, mTLS-exempt, `[service]` rejected; **`[D7]` the isolation
claim, its in-feature items (1–3, 5–6) and their slice placement, and mount-ns being
#258's**; **`[D8]` storage-by-role — virtiofs for volumes, the `[[vm.volume]]` surface,
conditional `shared=on`, `--cache=never`, `--sandbox=namespace` fail-closed, and the
daemon's cgroup/netns placement**; and **the `JobEnvelope` V1→V2 bump** (`[G4]`, user-ruled).

**Nothing outstanding.** Both items that needed the user were ruled on 2026-08-02 — see
§ Blockers. DISCUSS is closed and ready for the DESIGN handoff.

---

## `[REF]` Blockers — all ruled on 2026-08-02; none outstanding

The four blockers this document raised on 2026-08-01 have all been decided by the user, as
has the mount-namespace question (`[B5]`). Every deferral above resolves to a real issue
number; no hand-wavy forward pointer remains.

| Was | Ruling (2026-08-02) | Resolves to |
|---|---|---|
| `[B1]` Service-kind VM needs tap-in-netns + guest-reachable probes, untracked | **Approved, filed** | **[#257](https://github.com/overdrive-sh/overdrive/issues/257)** — cited by US-VM-6 alongside #222 |
| `[B2]` Fill in GH #42's `TODO` Acceptance placeholder | **NOT approved. #42 is left alone.** The dependency is dropped outright — this document is the ratified scope, and **no replacement forward pointer is written in its place.** | — (closed, no successor) |
| `[B3]` No isolation guarantee claimed; compensating work untracked | **Approved AND rescoped.** Six concrete items folded into this feature; the jailer remainder tracked. | **[#258](https://github.com/overdrive-sh/overdrive/issues/258)** — and `[D7]` is the claim |
| `[B4]` OCI → rootfs image factory deferral (I-3), untracked | **Approved, filed** | **[#259](https://github.com/overdrive-sh/overdrive/issues/259)** |
| `[B5]` GH #258's body listed the **mount namespace** as in-scope for *this* feature | **Approved — the argument was accepted as stated.** A bare `unshare(CLONE_NEWNS)` is a hollow claim (private mount *table*, unchanged host filesystem *view*); the version with real security value is `pivot_root` + `MS_PRIVATE` + bind-mounted artifacts + `mknod`/bind of `/dev/kvm`, which **is** the chroot half of the jailer #258 owns and that Landlock was chosen instead of. **#258 amended 2026-08-02; mount-ns is #258's, not this feature's.** | **[#258](https://github.com/overdrive-sh/overdrive/issues/258)** |
| — (raised by the `[D8]` amendment) | **`virtiofsd` hardening in #258 is now UNCONDITIONAL**, no longer *"only if we adopt virtiofs"* — `[D8]` puts virtiofs on the roadmap. **#258 amended 2026-08-02.** The lifecycle/posture boundary is the table in `[D7]`. | **[#258](https://github.com/overdrive-sh/overdrive/issues/258)** |

### Both remaining items ruled 2026-08-02 — nothing outstanding

1. **Slice-04 file hygiene — DONE.** `slices/slice-04-resources-size-the-vm.md` was
   **deleted** by the orchestrator on 2026-08-02. `slices/slice-05-resources-size-the-vm.md`
   is the sole live brief for the resources slice. No stub, marker, or shim remains — the
   `slices/` directory now holds exactly six files, one per slice.

2. **Slice 04 (volumes) STAYS in this feature.** User ruling, 2026-08-02, in response to the
   direct question *"shouldn't volumes / virtiofs be deferred to a separate feature?"* —
   answered *"just keep it in this."* This ratifies DISCUSS's own position and its reasoning:
   `I-6` was an intake decision of *this* feature that had already been dropped once, and
   lifting it out would functionally re-defer it — the same failure being corrected.

   **The pre-authorised lift trigger is hereby retired, not merely unmet.** DESIGN does
   **not** re-run this question at the handoff. If Slice 04's budget later exceeds
   `[D8f]`'s 9-day upper band, that is a **re-slicing** question inside this feature
   (split the volume slice), not a lift-to-a-new-feature question. Re-opening the lift
   requires a fresh user ruling, because the operator-facing consequence is the same one
   named at intake: a VM class that computes but cannot deliver.

---

## Wave: DESIGN — system / infrastructure scope (Titan, 2026-08-10)

> **First of three DESIGN dispatches.** This section covers **infrastructure-level**
> architecture only: failure domains, host-state placement, dispatch latency, resource
> commitment, and substrate trust. The `Vmm` trait signature, `VmConfig` shape, spec
> parse surface, `TransitionReason` vocabulary and driver dispatch are **application
> architecture** and belong to the solution-architect dispatch that follows. The domain
> model (aggregate boundaries, restart-accounting rules) belongs to the ddd-architect
> dispatch. See § *Handoff* at the end.
>
> **Scope, per the user's 2026-08-10 ruling:** boot a VM through `overdrive serve` +
> `overdrive deploy`. Slices 01–05. Checkpoint/restore, persistent rootfs, warm pools,
> the chunk store and the guest agent's full protocol are **#96 / #97 / #100** and are
> not designed against here.
>
> **SSOT output:** `docs/product/architecture/brief.md` § *System Architecture* →
> *Cloud Hypervisor VM driver — host-process failure domain, per-allocation host state,
> and the VM substrate probe*, carrying decisions **SD-1 … SD-5** and the mandatory
> C4 Level 1 + Level 2 diagrams.

### `[REF]` Was system-level design warranted? — **Yes, for exactly five things; no, for everything else**

The dispatch explicitly licensed the answer *"little or no system design is required."*
That is **not** the honest answer here, but the reason is narrow and worth stating
precisely, because the temptation in the other direction is real.

**What is NOT here, and would have been over-engineering.** A single node booting one
VM per allocation has **no** placement problem, **no** sharding, **no** replication,
**no** consistency model, **no** cache tier, **no** CDN, **no** queue, and **no**
request traffic to estimate QPS or bandwidth for. Every one of those was considered and
rejected as absent. The classic back-of-envelope (DAU → QPS → servers) does not apply;
the estimation that *does* apply is **resource commitment** — memory, disk, and
control-plane latency — and it is done below.

**What IS here.** Five node-level infrastructure properties, each of which (a) is a
failure-domain, state-placement, latency-budget, resource-commitment or substrate-trust
decision, (b) has a **default outcome that ships if nobody decides**, and (c) that
default is wrong.

| ID | Decision | The default that ships if undecided |
|---|---|---|
| **SD-1** | Hypervisor is outside `serve`'s failure domain; VM host state is reclaimed by a **`Reconciler` (Bar 2)** whose boot-epoch pass **reaps, never adopts** | `ExecDriver`'s inherited behaviour: the VM survives, becomes unstoppable through the driver, and the observation store claims it terminated — while holding GiB of guest RAM |
| **SD-2** | Per-allocation host state spans **two filesystems with different invalidation semantics** | Everything under one run dir → either reflink silently degrades ~260×, or the marker cannot distinguish "serve restarted" from "host rebooted" |
| **SD-3** | Blocking `start()` bounded **inside the driver** by a three-way race | A serial, timeout-free dispatch loop parks on every VM boot; a VM that never beacons freezes convergence for the full deadline |
| **SD-4** | `memory.max` = `memory_bytes` **+ reserve** | `memory.max` == guest RAM ⇒ every VM is cgroup-OOM-killed and reported as `signal: 9` |
| **SD-5** | `Vmm::probe()` at boot; **composition-gated hard refusal** — a substrate *lie* refuses the node, a capability *absence* does not | Seven substrate lies each surface two layers from their cause |

**The strongest single argument that this was warranted:** three of the five defaults are
*silent*. SD-2's reflink degradation, SD-4's OOM misclassification and SD-5's rows 1–2
produce no error anywhere — they would have been discovered in production, not in
DELIVER.

---

### `[REF]` Estimation — the numbers this design rests on

Figures are measured (spike, CH v53.0, bare-metal x86_64) or derived from a cited code
fact. **Four quantities this design leans on are NOT measured and are labelled as
assumptions in place** — Cloud Hypervisor's failure-to-exit latency, `ExecDriver::start`'s
absolute latency, SD-4's `reserve` on the boot path, and (**added after adversarial
review, previously labelled nowhere**) `--memory shared=on` on **aarch64**. All go on
DELIVER's measurement list. The `shared=on` premise is the one with a scoping
consequence rather than a tuning one:

> **A-3 — `shared=on` is measured on x86_64 only.** P6 exercised the volume path on
> x86_64; `findings.md`'s verdict table says *"aarch64 still unmeasured"* and
> `wave-decisions.md` carries it under Still-open. **Slice 04 nonetheless designs the
> volume path for both shipping arches** — the `shared=on` derivation, `rlimit_fsize =
> max(rootfs, guest RAM)` (C-6) and the volume payload. **Sensitivity: if `shared=on`
> misbehaves on Arm metal, Slice 04 is x86_64-only until measured.** What gates is the
> volume *capability*, not the driver — Slices 01–03 never turn `shared=on` on.

**Boot latency and control-plane stall (SD-3).**

```
guest reaches /init             0.730 – 0.746 s   12/12 runs, 16 ms spread   (P1, metal)
ready beacon at host            ~1.1 s                                        (P2, metal)
same, nested aarch64            ~8.7 s            (module loads + nesting)    (P2, env A)
tick cadence (DEFAULT_TICK_CADENCE)
                                100 ms            reconciler_runtime.rs:1274
ExecDriver::start today         fast, UNMEASURED  cgroup scope create + 2 limit writes
                                                  through the CgroupFs port, then spawn
                                                  (driver.rs:172 — "no direct tokio::fs::*
                                                   calls from driver.rs", ADR-0054 §D5)

⇒ one VM start ≈ 11 ticks of wall clock on metal, ≈ 90 nested
⇒ B VM starts in one drain batch stall the WHOLE loop for B × 1.1 s
⇒ worst case (VM boots, never beacons) = pending_vm_starts × boot_deadline
     at D = 30 s, 5 such VMs  ⇒  ~150 s of total convergence freeze
```

Only the **order-of-magnitude gap** between an exec spawn and a guest boot is load-bearing
here, and that gap is not in doubt; the exec figure's absolute value is not used in any
decision.

**The VMM-exit arm's payoff rests on an assumption, stated as one.** A bad kernel /
unloadable rootfs / Landlock denial / OOM terminates `cloud-hypervisor` **without ever
producing a beacon**, so without that arm each costs the full deadline. The arm is
expected to resolve them far below any plausible deadline — but **the spike recorded CH's
exit *status* and never its exit *latency*, so no number is claimed.** If that assumption
is wrong — if CH's failure-exit approaches *D* — **option B's advantage over option A
collapses and option C should be re-opened.** That sensitivity is the reason the
measurement is mandatory in DELIVER rather than nice-to-have.

**Disk (SD-2).**

```
reflink clone, 4 GiB source     0.015 s   +    0 MiB   extents confirmed shared  (P4)
full copy, same source          3.970 s   + 4096 MiB                            (P4)
                                ⇒ ~260× and free in space
1 GiB + 64 MiB volume pair      ~9 ms     868 KiB actual / 1.0 G apparent        (P6)

Leak budget without GC: proportional to guest WRITES, not image size.
  100 leaked clones of a 2 GiB image, 50 MiB dirtied each  ≈ 5 GiB   (not 200 GiB)
Unbounded over the appliance lifetime — clones sit on the PERSISTENT filesystem
(mandatory for reflink) and do NOT self-clear on host reboot.
```

**Memory (SD-4).**

```
VmRSS at beacon, noshare        276,888 kB   with 128 MiB deliberately touched  (P5)
  RssAnon 273,232 kB · RssShmem 4 kB · no memfd · 9 threads
same VM, shared=on              265,456 kB   RssAnon 852 kB / RssShmem 260,952 kB
  ⇒ shared=on RECLASSIFIES the footprint and lands ~11 MB LOWER — it does not inflate it
  ⇒ x86_64 ONLY (P6/P5); aarch64 unmeasured — assumption A-3 above

cold-boot residency               NOT MEASURED, workload-dependent
  the only cold-boot datapoint is the 276,888 kB above, with 128 MiB deliberately
  touched — nowhere near full residency. Residency trends toward the declared
  figure over a run (the guest's own page cache retains what it reads) but neither
  the rate nor the ceiling is measured on this path.
  (An earlier draft wrote "guest RAM becomes Private_Dirty in full within ~2.5 s
   (P13/P14)" here. That is P13's `ondemand`-RESTORE uffd backfill — a restore-path
   property of a banked probe — applied to the COLD-BOOT path this feature ships,
   and the 276,888 kB datapoint above refutes the generalisation. WITHDRAWN.
   SD-4 never rested on it: the charging argument below is independent of when
   residency happens.)

VMM footprint at zero guest residency
                                12,136 kB    RssAnon 7,580 / RssFile 4,556 / RssShmem 0
                                             (t=0 of an ondemand restore, guest touching
                                              nothing — includes the uffd-handler thread)
steady-state RSS above a 2 GiB guest
                                 5,532 kB    VmRSS 2,102,684 − 2,097,152
                                             (of which ~4.5 MB is the binary's text —
                                              RssFile sits flat there)

the cgroup charges MORE than RSS reports: host page tables for the guest mapping are
charged via memory.stat `pagetables` and are INVISIBLE to RSS.
memory.max = memory_bytes AND guest RAM = memory_bytes  ⇒  over limit by construction.
```

**The reserve has partial floors and no measured boot-path value — and RSS structurally
cannot supply one.** *(Corrected after review: an earlier draft computed
`Rss − Private_Dirty = 4,540 kB` and called it "~4.4 MB above guest RAM … the vCPU-stack
+ ring + API-server + text + page-table bundle, isolated." Both halves were wrong.
`Private_Dirty` is 2,098,144 kB — already **992 kB above** the 2 GiB guest — so it is the
wrong subtrahend; and the 4,540 kB remainder matches `Shared_Clean` (4,536 kB), i.e. it is
**the binary's text alone**, which `findings.md` says in as many words. The vCPU stacks
and rings are private-dirty and sit *inside* the larger term.)* The two honest floors are
in the block above; page tables are in neither, because RSS cannot see them. Shipping a
guessed constant between them would be exactly the "magic version floor" failure of intake
precedent warning #7. **DELIVER measures the reserve against a real boot via
`memory.current` / `memory.stat` — not RSS — and cites the measurement.**

**Contrast with a process workload, which is the whole reason this matters.** An exec
workload's RSS is typically far below its `memory.max` and stays there; the kernel
overcommits and, on pressure, kills one process. A VM's declared RAM is a **standing
claim**: its host-resident share grows as the guest touches pages and does not shrink back
(the guest's own page cache retains what it reads), so residency trends toward the declared
figure over the run. **How fast and how far is workload-dependent and unmeasured on the
cold-boot path** — but the direction is the point, and it makes over-admission soft for
processes and **hard for VMs**. Note that SD-4's decision does not depend on this
paragraph at all: the VMM's own footprint plus invisible page tables are charged on top of
whatever is resident, from the first byte, so `memory.max == guest RAM` is over its limit
by construction regardless of residency timing.

---

### `[REF]` The five decisions — options considered and trade-offs named

#### SD-1 — Failure domain: three options, and the default is the worst

| | **A. Inside the failure domain** (kill at `serve` shutdown / `kill_on_drop`) | **B. Outside + adopt at boot** | **C. Outside + REAP at the boot epoch** ← **recommended** *(and, per the bar correction below, the reap is the boot-epoch pass of a continuously-ticking reclamation `Reconciler`)* | **D. Outside, no boot pass** ← *today's default* |
|---|---|---|---|---|
| VM survives a `serve` restart | No | Yes | No | Yes |
| Exit status stays honest | Yes | **No** — see below | Yes | **No** |
| Stoppable after restart | n/a | Yes | n/a | **No** — `Err(NotFound)`, swallowed by `let _ =` |
| Guest RAM leak on restart | none | none | none | **full guest RAM, indefinitely** |
| Survives `serve` SIGKILL / host crash | **No** — the mechanism is in the dying process | Yes | Yes | Yes |
| Mechanisms to keep correct | 2 (shutdown + a boot pass anyway) | 2 + a guest reconnect protocol | **1** | 0 |
| Cost | VM dies on every upgrade | Not achievable at this scope | VM dies on every upgrade | — |

**B is unavailable, and the reason is measured rather than assumed.** P2 shows the guest
agent opens **one** guest→host vsock connection and carries both the READY beacon and
`EXIT 7` over it (`separate_reads=2`, then `EOF`). The host end is a `UnixListener` inside
`overdrive serve`. Kill `serve` and the accepted connection dies; the ~200-line PID-1 agent
does not re-dial. P12 independently confirms the hazard's shape post-restore: *"one
post-restore `send()` succeeds while being silently discarded"* — so an adopted VM could
even *appear* to report and not have. **An adopted VM is a VM whose ending can never be
honestly classified**, which is `[D3]`, the feature's north star, inverted. A reconnect
protocol is GH #100's guest agent, not this feature's.

**A is rejected on the mechanism count.** A shutdown-time stop fails exactly when it
matters — SIGKILL, host crash, OOM — so the boot pass must exist regardless. Adding the
shutdown path buys a slightly gentler common case and a second thing to keep correct.
Chosen trade-off: **one mechanism, and it is the one that covers every case.**

**C's honest cost, stated:** every `overdrive serve` restart kills every running VM.
That is ~1.1 s of re-boot per VM on metal (measured), against a process respawn that is
orders of magnitude cheaper — a real operational cost on upgrades. It is accepted because the alternatives are "the exit
status becomes a lie" (B) or "GiB-scale unstoppable orphans" (D).

**C is specified as an idempotent observe → pure diff → converge, not as a four-step
imperative — and that distinction was a review finding, not an original strength.** The
first draft wrote the reap as *"kill the scope, unlink the run directory, write a terminal
row"*. An apply-once reap is the half-provisioned-resource bug
`.claude/rules/reconcilers.md` exists to prevent, reproduced in this design's own headline
decision. The corrected specification — observed surfaces, the authority rule when they
disagree, per-step idempotence, and the convergence of every partial-crash state — is in
`brief.md` § SD-1.

**The BAR, corrected after adversarial review: this is Bar 2, not Bar 1.** The first draft
asserted Bar 1 having run only the *workflow-disqualification* test and then reasoned **by
analogy** to `veth_provisioner::provision`. Those are two different tests, and the
Bar-1-vs-Bar-2 one — ***does `actual` drift while the system is up?*** — was never run. Its
honest answer is **yes**, from this design's own text: a clone leaked by a crash between
teardown steps; a scope or run directory stranded by a failed stop, leaving the VM
**unstoppable until the next `serve` restart**; and SD-2's clone-leak GC being boot-only,
so a node whose `serve` never restarts never sweeps a leak SD-2 itself calls unbounded over
the appliance's lifetime. Only continuous convergence repairs any of those while the node
is up. **User ruling, 2026-08-11: reclamation ships as a registered `Reconciler` (Bar 2).**
`veth_provisioner::provision` remains the precedent for the *shape* (observe → pure
`converge_steps` → idempotent execute), not for the *bar*.

**And one argument for Bar 2 that does NOT hold — recorded so it is not resurrected.**
*"The VMM is detached, so `serve` cannot see its mid-run exit"* is **false**. `setsid(2)`
detaches the session and process group, **not parentage** (`driver.rs:355`, `:372-377`):
the VMM stays a child, and the exit watcher's `wait()` fires on any mid-run VMM death
including SD-4's cgroup OOM. What forces Bar 2 is the *host-state ensemble* around the
process, which no `wait()` observes.

**Three further constraints fall out of C, all non-obvious:**

1. **Ordering — reclaim before `adopt_on_restart_recovery`** (`lib.rs:2117-2145`). Both
   walk `overdrive.slice/workloads.slice/*/cgroup.procs`. Adopt-first adopts a netns slot
   for an allocation about to be destroyed, and that netns then escapes the same pass's
   orphan GC. Reap-first leaves an empty scope, so the adopt pass correctly treats the
   netns as orphaned and reclaims it. **Bar 2 changes the mechanism, not the constraint:**
   a broker-driven reconciler has no bootstrap sweep, so "before adopt" cannot be a tick.
   The boot-epoch convergence is driven **synchronously inline** in the boot sequence
   (same pure diff, same executors — one implementation, two drivers), carries the
   rmdir-settled obligation, and the reconciler is **registered after** the boot passes so
   no tick can interleave with them. Registration is **not** gated on the `Vmm` adapter
   being composed — a node where CH was uninstalled still holds VM host state that nothing
   else will reclaim.
2. **The reap must not consume restart budget.** `RESTART_BACKOFF_CEILING = 5`
   (`workload_lifecycle.rs:23`). Six `serve` restarts would otherwise drive **every** VM
   workload on the node to `RestartBudgetExhausted` — a node-wide terminal cascade caused
   by routine upgrades. This is an availability property, not a cosmetic one.
3. **The two regimes are not the same rule, and conflating them kills live VMs.** The
   **boot epoch** may reclaim *every* VM allocation with surviving host state, non-terminal
   ones included — their vsock channel died with the previous `serve`, so their ending can
   no longer be classified. A **steady-state tick** may reclaim only host state whose
   allocation is terminal or unknown, plus artifacts stranded by a failed stop. The
   discriminator (*"is this `serve` supervising this VMM?"*) must be an **observed input
   hydrated into `actual`** — the driver's live-handle set — never a marker the reconciler
   stamped on its own emit path, which is `reconcilers.md`'s fingerprint-as-diff
   anti-pattern and here would be load-bearing on whether a healthy VM is killed.

**And one honesty constraint:** the reap is occurrence-bearing
(`.claude/rules/development.md` § *"A convergent record cannot answer 'did it happen'"*).
It must reach the durable `LastTerminated` snapshot + `restart_count` per ADR-0078, not
merely converge back to `Running`.

**What Bar 2 costs — stated, because a bar change is not a relabelling.** A registered
`Reconciler` brings a `State` hydrated from **host** surfaces (cgroup tree, run-directory
root, staging directory) plus the intent-side `WorkloadDriver::Vm` join; a `View` that
should be **field-less** per ADR-0079; registration in `run_server`; ESR (progress +
stability) specifications; and DST reachability. And because a reconciler is pure and
mutates only through the action-shim (ADR-0023), reclamation needs **≥1 new `Action`
variant plus its executor surface** — which falsifies the domain wave's *"nothing
structural — no new `Action` variant"*. Handed to the domain architect below. The
plan/execute split is a **reshape, not a loss**: `reconcile` is the pure diff returning the
plan, the plan's steps become Actions, the executors are the impure half.

**Does this found the shared "host/node infrastructure reconciler" model, or ship a bespoke
fifth? — It ships a concrete instance and sets the precedent; it does NOT found the shared
abstraction.** `reconcilers.md:274-284` names four deferred Bar-2 promotions (#197 veth,
#198 cgroup hierarchy, #199 XDP attachment, #234 inbound-TPROXY routing) as sharing that
machinery, with **#197 as its candidate home**. Generalising an abstraction from its first
instance — inside a driver feature, across four resource classes with genuinely different
shapes — is speculative generality, and it is the same scope creep SD-3 refuses for the
dispatch path (deferral D-1). **The consequence is symmetric and both halves are stated:**
gained, no cross-feature abstraction invented on one datapoint and the feature stays sized
to a loop `serve` + `deploy` actually drives; paid, this becomes a **fifth** site for #197
to migrate, with a real risk that its shape is copy-pasted four times before anyone
generalises. **The mitigation is a design obligation, not a hope** — the host-observation
hydration must be a named, separable step producing a plain observed-state value, and the
diff a pure function over it, so #197's work is a refactor of an existing seam rather than
a rewrite. This is nonetheless the first in-tree reconciler whose `actual` comes from host
state rather than from the intent or observation stores, so #197 inherits a worked example
of the exact hydration problem it exists to solve.

**Four new acceptance obligations follow from the bar change** (DISTILL's, named here so
they are not lost): mid-run drift repair **without** a `serve` restart; a live VMM whose
allocation row is terminal killed at tick *N*; the boot-epoch pass still settling every
`rmdir` before `adopt_on_restart_recovery` reads the tree; and a supervised, non-terminal
VM **surviving** every steady-state tick (the safety half — without it the reconciler's
first regression kills healthy VMs).

#### SD-2 — Host-state placement: the two-filesystem split is forced, not chosen

The tmpfs-vs-persistent split is **derived from two measured constraints pulling in
opposite directions**, which is why it reads as a decision rather than a detail:

- **Pull toward tmpfs:** the marker must distinguish "`serve` restarted, VM survives"
  from "host rebooted, VM is gone." tmpfs gives that for free — it survives the former
  and clears on the latter. No durable `(alloc → pid)` record exists anywhere: host PID
  is persisted in **no** observation row (`AllocationHandle.pid`, `traits/driver.rs:241`,
  is in-memory only). So the run directory *is* the durable `alloc ↔ VM` join.
- **Pull toward the master's filesystem:** reflink is intra-filesystem. increment-f
  staged images in `/run` and got `Invalid cross-device link`; with `--reflink=auto`
  (coreutils ≥9's default for plain `cp`) the same command would have done a **full copy
  with no error**, evaporating P4's ~260×.

Both pulls are absolute, so the state splits. **Sockets and logs on tmpfs; disk images on
the master's filesystem.** The alternative — one location — loses one property or the
other, silently in both directions.

**Directory exclusivity is a confinement property, not tidiness.** P5, *the vsock-UDS
Landlock gap*: the
vsock UDS needs a per-VM `--landlock-rules` **directory** grant (CH auto-derives rules for
`--kernel` / `--disk` / `--serial file=` / `--api-socket` but not for the socket it binds
itself; the rule cannot name the socket path because CH validates existence at
config-parse time, before the socket exists; a read-only rule fails). The directory grant
*is* the ruleset entry, so anything else in that directory is inside the VM's writable
confinement.

**Not an accepted cost — an assigned one.** The first draft named the clone leak,
quantified it, and routed it to no slice, no AC and no deferral. That is an
unbounded-growth failure mode on a target with no operator shell, and "stated in prose" is
not an owner. **It is folded into SD-1's reclamation reconciler**, which already walks the
allocation set: any clone whose allocation is terminal or unknown is swept. Two
consequences worth naming: **the clone filename must carry the allocation id** (after a
host reboot the tmpfs run directory is gone, so the filename is the only remaining
attribution), and **Slice 03's *"no leaked … rootfs copies after terminal states"* does not
cover this case** — there is no allocation left to key a terminal-state GC off.

**And this paragraph is where the Bar-2 ruling pays for itself.** Under the first draft's
Bar 1 the sweep ran only at boot, so an unbounded-over-lifetime leak was bounded by the
node's *restart rate* — and a node whose `serve` never restarts never swept at all, on a
target with no operator shell. Under SD-1's registered `Reconciler` the sweep runs at the
tick cadence instead. Nothing else about this decision changes.

#### SD-3 — Bounding the blocking start: three options

| | **A. Block, unbounded** (`[D2]` as written) | **B. Block, bounded in the driver** ← **recommended** | **C. Async readiness seam** |
|---|---|---|---|
| Shim change | none | none | **required** — exactly what `[D2]` avoids |
| Common-failure stall | full deadline | **CH's exit latency — assumed ≪ D, UNMEASURED** | ~0 |
| Worst-case stall | `pending × D` | `pending × D` | ~0 |
| Re-opens the `Running` lie | no | no | **yes, if done wrong** |
| Cost | control plane parks on every bad VM | residual stall named, not removed | a second feature |

**B is recommended and both its residual and its assumption are stated rather than
hidden.** The three-way race — ready beacon **‖** VMM process exit **‖** boot deadline —
costs nothing structurally, and the VMM-exit arm also carries CH's stderr, which is where
`[D5]`'s "name the real problem" diagnosis lives, so it does double duty regardless of
latency. **What is not established is the size of the win**: it is proportional to how
fast CH exits on a failed boot, which the spike never measured. **B's margin over A is
therefore conditional, and the condition is on DELIVER's measurement list.**

**C is very likely the right end state** and is named so it is not re-derived from
scratch later. It is out of scope because a shim change re-opens the seam `[D2]` was
chosen to close.

**Explicitly NOT added: a semaphore or queue-depth bound on `StartAllocation` dispatch.**
The dispatch path is fully serial today (`lib.rs:2427-2477`, `action_shim/mod.rs:690`),
`EvaluationBroker` is unbounded (`overdrive-core/src/eval_broker.rs:59-104` — `pending` is
bounded only by *distinct targets*, and `cancelable` grows until `reap_cancelable` runs),
and `TickContext.deadline` is constructed and read by **no production reconciler and no
runtime code** (the DST invariant harness does read it). Every one of those is a
pre-existing control-plane-wide property; changing them inside a driver feature is scope
creep. Surfaced as a deferral below.

#### SD-4 — Where the VMM's overhead is charged: two options

| | **A. Guest RAM = `memory_bytes` − reserve; `memory.max` = `memory_bytes`** | **B. Guest RAM = `memory_bytes`; `memory.max` = `memory_bytes` + reserve** ← **recommended** |
|---|---|---|
| What the operator's number means | total host cost | **the guest's RAM** |
| Consistency with `[resources]` today | closer (it is a cgroup limit) | the host pays more than declared |
| Operator surprise | `memory_bytes = 512 MiB` yields a ~460 MiB guest | none |
| Industry shape | — | matches every VM platform (guest RAM is what is sold) |
| Scheduler interaction | reserves the true cost | reserves `memory_bytes`, host commits more |

**B, with the scheduler discrepancy named and not fixed.** The workload observes the
number the operator wrote — that is the operator-meaningful quantity for a VM. The
scheduler discrepancy is real but is a rounding error on a fiction: `baseline_nodes_phase1()`
hardcodes 4000 mCPU / 8 GiB (`reconciler_runtime.rs:3204-3216`), the allocation set is
filtered to one workload before it reaches `schedule()` (`reconciler_runtime.rs:2871`),
and `AllocStatusRow` carries no `Resources` at all. Correcting the reserve's contribution
to an accounting model that already admits 100 × 8 GiB onto an 8 GiB fiction node would be
precision without accuracy.

**What SD-4 does buy, and it is the point:** with the VMM in the allocation's own scope
(Slice 01, `[D7]` item 5) plus a correct limit, a VM that genuinely exceeds its declared
memory is killed by the **cgroup** OOM killer inside its own scope, rather than the host
OOM killer choosing an arbitrary victim. The blast radius goes from *the host* to *this
allocation*.

**Named and not fixed:** `TransitionReason::OutOfMemory { peak_bytes, limit_bytes }` is
defined (`transition_reason.rs:164-169`) and has **no production emit site** — it is
constructed only in archive-roundtrip and snapshot tests — so a cgroup OOM
still surfaces as `Failed / WorkloadCrashedImmediately { signal: 9 }` —
indistinguishable from `kill -9`, with no mention of memory. Constructing it needs a
`memory.events` subscription, which is out of scope. Deferral below.

#### SD-5 — Boot refusal vs capability refusal

> **This decision was revised after review, because its evidence was inverted.** The first
> draft recommended a *capability refusal* — probe fails, node still boots, `VmDriver`
> composed in a refusing state — and justified it with *"the `EbpfDataplane` disposition
> (`lib.rs:1683` emits `health.startup.refused` at `warn!` and boot continues)."*
> **That reading is false.** `lib.rs:1681-1693` emits at `warn!` **and then
> `return Err(ControlPlaneError::DataplaneBoot(..))`**; the comment above it says *"refuse
> to boot"* verbatim. The `warn!` level is a logging choice, not a disposition. **There is
> no in-tree precedent for a probe that fails and lets the node start** — all six
> (`ViewStore`, `JournalStore`, `CgroupFs`, `MtlsEnforcement`, `MtlsResolve`,
> `cgroup_preflight`, plus `ProbeRunner` at `probe_runner_boot.rs:63` and `DnsResponder`
> at `lib.rs:2253`) refuse; the one exception, `JournalStore`, is **never called at all**
> (deferral D-4). The claim followed an inference from a log level rather than the code;
> correcting it changes the recommendation.
>
> **The corrected recommendation is stronger than the original, not a retreat.** Option C
> is not merely *analogous* to an in-tree pattern — `MtlsEnforcement::probe` and
> `MtlsResolve::probe` sit inside `if compose_mtls` and are composition-gated hard refusal
> shipping today. SD-5 applies that pattern to a second optional subsystem.

| | **A. Hard boot gate, unconditional** | **B. Capability refusal** *(originally recommended — withdrawn)* | **C. Composition-gated hard refusal** ← **recommended** |
|---|---|---|---|
| Node with no CH installed | **cannot start** | starts | **starts**; `[vm]` rejected at admission |
| CH present, substrate lies | refuses to boot | starts, every VM deploy fails | **refuses to boot** |
| Precedent | every Earned-Trust gate in tree | **none — the cited one was misread** | **already ships**: `MtlsEnforcement::probe` (`lib.rs:1988`) and `MtlsResolve::probe` (`:2021`) sit inside `if compose_mtls` (`:1935`) — composed conditionally, and once composed, a failing probe refuses the node |
| Risk | makes CH a hard dependency of every node | a misconfiguration hides as a runtime failure | composition gate must key off something observable |

**C, and the split is the substance of it.** *"CH is not installed"* and *"CH is installed
and the staging filesystem cannot reflink"* are different facts. The first is a node that
does not offer a capability — not a fault, and refusing to boot on it would make Cloud
Hypervisor a hard dependency of every node in the fleet. The second is an operator
misconfiguration, and degrading it to "every VM deploy fails at runtime" buries it exactly
as the seven substrate lies bury themselves. **Fail-closed applies to the fault, not to
the absence** — and with the split drawn there, C conforms to all six precedents on the
half that Earned Trust is actually about.

**The composition gate keys off an observable, not a new operator knob.** The presence of
the hypervisor binary is the configuration — the same shape as
`compose_mtls = config.dataplane_override.is_none()`. Unstated knobs are out of scope by
default (CLAUDE.md), and DISCUSS scoped no node-capability config. **How the composition
root expresses the gate is a solution-architect decision**; what is decided here is that a
substrate *lie* refuses the node and a capability *absence* does not.

**The gate's inverse hazard is named in `brief.md` § SD-5 rather than left to be
discovered:** under this rule, **installing the `cloud-hypervisor` binary can flip a node
from booting to refusing to boot** if its staging filesystem cannot reflink. That is
correct behaviour — a node advertising VM support on a substrate that cannot honestly
deliver it should not run — but it lands at the next `serve` boot, and an unstated version
of it reads as an unexplained boot failure after an unrelated package update.

**The staleness risk is closed by self-application (principle 9, recursively):** rows 1
and 2 of the lie table keep **per-launch** enforcement — `--reflink=always` (or `FICLONE`
directly) and an explicit `image_type=raw` on every `--disk` — so a remount, a package
upgrade or a different staging path makes the *launch* refuse rather than silently
degrade, even when the boot probe passed. The probe is the gate; the per-launch flag is
the proof the gate is still honest.

**Note this inverts P4's own guidance, deliberately.** P4 concluded the `--reflink` flag
is *redundant* because coreutils ≥9 defaults to `auto`. That is true for **performance on
a capable filesystem** and false for **failing loudly on an incapable one**: `always`
fails with `Invalid cross-device link`; `auto` silently costs 3.97 s and 4 GiB. The flag is
redundant for speed and load-bearing for honesty.

---

### `[REF]` Reuse Analysis — **hard gate**

Every existing component whose responsibility overlaps this design. `CREATE NEW` requires
evidence that extending is impossible.

| # | Existing component | Overlap | Verdict | Evidence |
|---|---|---|---|---|
| 1 | `veth_provisioner::adopt_on_restart_recovery` (`lib.rs:2117-2145`, ADR-0061) | boot-time reconciliation of host-resident per-alloc state | **EXTEND** | Same boot pass, same call site, same `overdrive.slice/workloads.slice/*/cgroup.procs` walk, same gate position. A *separate* pass would race it (SD-1 ordering constraint). Its own comment already names the gap: *"`WorkloadLifecycle::reconcile` does NOT re-drive a Running survivor (SPIKE-B), so this dedicated boot pass is the ONLY trigger."* **Under Bar 2 this row covers the boot-epoch drive only** — the steady-state ticks are registered after the boot sequence and have no adjacency to this pass. |
| 1b | Reconciler runtime — `Reconciler` trait, `ViewStore`, `EvaluationBroker`, `register` + `spawn_convergence_loop` (ADR-0035 / ADR-0036) | the Bar-2 machinery SD-1 now needs | **REUSE UNCHANGED** | The runtime already provides pure `reconcile`, typed `View` persistence, bulk-load + write-through and registration. SD-1 adds a **registrant**, not runtime machinery. **What is genuinely new is the hydration source** — this is the first reconciler whose `actual` comes from host surfaces rather than the intent/observation stores; the runtime's `hydrate_actual` arm is extended per-reconciler by construction, so this is an addition, not a change. |
| 1c | `action_shim::dispatch` + `Action` (ADR-0023) | the impure half of reclamation | **EXTEND** | A reconciler mutates only through Actions, so reclamation needs ≥1 new variant plus its executor arm. Extending the shim is the *only* sanctioned path — a reconciler calling an executor directly is the boundary violation ADR-0023 exists to prevent. Variant naming is the domain architect's; the executor's bounded-change contract is the application architect's. |
| 2 | `cgroup_preflight::run_preflight` (ADR-0028, `lib.rs:1308`) | host-capability refusal at boot | **EXTEND** | Same "BEFORE any on-disk side effects" seam **and the same disposition** — SD-5 option C refuses the node on a substrate lie, uniform with this. The only divergence is *when the gate is composed at all*, which is a composition-root question, not a change to this component. |
| 3 | `ViewStore::probe` / `JournalStore::probe` / `CgroupFs::probe` / `MtlsEnforcement::probe` / `MtlsResolve::probe` + `health.startup.refused` | Earned-Trust boot gate | **EXTEND** | **Five port traits** carry `probe()`; `traits/cgroup_fs.rs:48-55` states the contract verbatim (*"wire then probe then use"*). `Vmm::probe()` is the sixth trait instance of an established pattern. *(`EbpfDataplane::probe` is an **inherent method on a struct**, not a trait method — the `Dataplane` trait has no probe. It is precedent for the disposition, not for the trait surface.)* |
| 4 | `CgroupManager` create-scope → write-limits → enrol-PID (ADR-0026, ADR-0054) | VMM cgroup placement + `memory.max` | **EXTEND** | SD-4 changes the **value** written, not the mechanism. `write_resource_limits(&scope, &Resources)` (`cgroup_manager.rs:346-360`) is unchanged; the reserve is a derivation upstream of it. |
| 5 | `ExecDriver`'s `pre_exec` + `setns(CLONE_NEWNET)` (`driver.rs:389-397`, `:449-465`) | VMM netns entry | **REUSE VERBATIM** | Already ratified — feature-delta § Slice 01 cost note: *"copied, not designed."* |
| 6 | `spawn_exit_watcher` (`driver.rs:810-831`) | watching the VMM process | **EXTEND** | Same shape — own the `Child`, `wait()`, classify (`:829-831`), park on the Running-confirmed gate. What differs is the **classification input** (`[D3]`: the agent's report, never the VMM's `WEXITSTATUS`), which is a substitution inside the existing structure. |
| 7 | `exit_observer::classify` + `ExitKind` + `WorkloadLifecycle` restart/backoff | VM exit classification and restart | **REUSE UNCHANGED** | Slice 03's stated learning hypothesis, and this design does nothing to disturb it. SD-1 adds one reason variant on the reap path and one budget exemption; neither changes the classifier. |
| 8 | `AllocationHandle { alloc, pid: Option<u32> }` (`traits/driver.rs:238-242`) | driver handle | **REUSE UNCHANGED** | `pid` already models the VMM's PID with no shape change. |
| 9 | `spawn_convergence_loop` / `action_shim::dispatch` / `EvaluationBroker` | dispatch topology, concurrency, timeouts | **NO CHANGE — deliberate** | SD-3 bounds the blocking **inside the driver** instead. Extending here means adding a semaphore, a per-action timeout, or `tick.deadline` enforcement to a control-plane-wide path — scope creep into a driver feature. Surfaced as deferral D-1. |
| 10 | `scheduler::schedule` / `baseline_nodes_phase1` / `NodeHealthRow` | admission control, node capacity | **NO CHANGE — named as a gap** | Pre-existing and structural: hardcoded 4000 mCPU / 8 GiB, per-workload alloc filtering, no `Resources` on `AllocStatusRow`, and `TransitionReason::NoCapacity` has **no production emit site** (tests only; the live `PlacementError::NoCapacity` at `scheduler.rs:70` is a different type and must not be conflated). VMs make it materially worse but do not cause it. Deferral D-2. |
| 11 | `TransitionReason::OutOfMemory` (`transition_reason.rs:164-169`) | OOM diagnosis | **NO CHANGE — named as a gap** | Defined; **no production emit site** (constructed only in archive-roundtrip and snapshot tests). The right variant for an SD-4 overrun, but constructing it needs a `memory.events` subscription. Deferral D-3. |
| 12 | `TcpProber` / `HttpProber` / `ExecProber` (the *prober traits*) | readiness | **NO REUSE — different concept** | These are *runtime workload health checks*; `traits/prober.rs:219` says so explicitly. And `[G6]` establishes that none of them reaches inside a guest. The VM readiness gate is the vsock beacon, not a probe. **Scoped to the prober traits deliberately:** `ProbeRunner::probe()` (`probe_runner_boot.rs:63`) is a *boot-time Earned-Trust gate*, not a health check, and belongs with row 3 — its `compose_and_probe_runner_gate` is the compose-then-gate shape SD-5 needs. |
| 13 | **Composition root driver dispatch** — `lib.rs:1422-1425`, the single hardcoded `Arc::new(ExecDriver::new(…))` | which driver an allocation reaches | **EXTEND** | ADR-0022 pre-committed the registry migration to *"the second driver class"*; ADR-0030 §6 pre-sanctioned per-driver-class spec types. **Included because SD-5 acts through it** — *"the `Vmm` adapter is not composed"* and *"`[vm]` rejected at admission"* are both statements about this line — and a decision cannot depend on a component the reuse gate never assessed. `[G1]` / system constraint 2 already make changing it the feature's pass/fail bar; the *shape* of the change (registry vs. match) is solution-architect's. |

**CREATE NEW — and every item was pre-ratified before this wave:**

| New | Why extending is impossible | Pre-ratified by |
|---|---|---|
| `Vmm` port trait in `overdrive-core` | Every host effect (process spawn, HTTP-over-unix, vsock, Landlock) is banned on `core`-class paths by dst-lint and unreachable from Tier-1 DST without a port. No existing port has hypervisor semantics. | **intake I-2**, system constraint 5 |
| `CloudHypervisorVmm` (`adapter-host`), `SimVmm` (`adapter-sim`) | The two halves of that port. | intake I-2 |
| `VmDriver` (`overdrive-worker`) | `ExecDriver` is exec-shaped throughout (`Child`, `WEXITSTATUS`, `send_sigkill_pgrp`); the VM path substitutes the classification source and adds the beacon race. Composition over the `Vmm` port, not modification of `ExecDriver`. | intake I-2, ADR-0029 |
| `Vmm::probe()` + the boot-epoch reap arm | Additions to existing patterns (#1, #3 above), not new subsystems. | — |
| **The reclamation `Reconciler` + its `Action` variant and executor** | **This wave's genuinely new surface, and it grew after review.** The Bar-2 ruling means reclamation is a registrant on the existing runtime (#1b) mutating through the existing shim (#1c) — so no new *machinery*, but a new *component* with a `State`, a field-less `View`, ESR specs and DST reachability. **It does not found the shared "host/node infrastructure reconciler" model** #197/#198/#199/#234 await; see SD-1. | user ruling 2026-08-11 |

---

### `[REF]` Contradictions found between the spike findings and the slices

Seven. Each is a slice statement that the spike's evidence refutes or leaves incomplete.
**None requires re-slicing; all are corrections to slice text and ACs.** Five (C-2, C-4,
C-5, C-6, C-7) rest on direct measurement; **C-1 rests on an inference and C-3 on
arithmetic, and both are marked as such** — a false contradiction would be worse than a
missed one.

| # | Slice says | Spike evidence | Consequence if unfixed |
|---|---|---|---|
| **C-1** | Slice 01: *"per-launch `cp --reflink=auto`"*; Slice 03: *"a fresh `cp --reflink=auto`"* | **Inference, not direct measurement.** Measured: `auto` at 0.015 s / +0 MiB on reflink-capable XFS (P4); `--reflink=always` *failing* cross-device (increment-f). `findings.md` states the degradation counterfactually — *"`--reflink=auto` … **would have** silently done a FULL COPY"*. The inference follows from documented coreutils semantics; **DELIVER measures `auto` cross-device to close it** | ~260× regression and +4 GiB per launch, with **no error anywhere**. Must be `--reflink=always`/`FICLONE` **and** same-filesystem staging (SD-2/SD-5). |
| **C-2** | No slice mentions `image_type` | `image_type=raw` is **mandatory on every `--disk` from v53**; auto-detect *"disables sector 0 writes"* and our bare-filesystem images fault, `panic=1` reboots, and the failure surfaces two layers from its cause (P10/P11) | A boot loop diagnosed as a corrupt rootfs. |
| **C-3** | Slice 01's four-step `start` shape writes resource limits; `[D1]`/Slice 05 derive guest RAM from the same `memory_bytes` | **Arithmetic on a measured fact**, not a measurement of the collision: the cgroup charges the VMM's entire RSS *plus* page tables it cannot see, and P13 measures `VmRSS` ~5.5 MB above a 2 GiB guest at rest (~11.9 MiB before guest RAM is resident) | `memory.max` == guest RAM ⇒ **cgroup OOM by construction**, reported as `signal: 9`. Bites from **whichever slice first derives guest RAM from `memory_bytes`** — Slice 01 writes the limits and must give the guest *some* size, so it either applies `[D1]`'s derivation (collision present at Slice 01) or hardcodes a default (a VM that ignores `[resources]`). Both are Slice 01 decisions. Slice 05 flags the *virtiofsd* allowance and not the hypervisor's own. |
| **C-4** | Slice 03 US-VM-7: ruleset *"derived from the paths this spec declares (kernel, per-launch rootfs copy, API socket)"* | CH **auto-derives** rules for exactly `--kernel` / `--disk` / `--serial file=` / `--api-socket`. The one path needing an explicit rule is the **vsock socket's containing directory** (P5, *the vsock-UDS Landlock gap* — cited by content because `findings.md` and `wave-decisions.md` number P5's three corrections in different orders) | The slice names the three paths that need no rule and **omits the only one that does**. Failure is `CreateVsockBackend(UnixBind(EACCES))`, which never mentions Landlock. |
| **C-5** | Slice 01 AC: *"`/proc/<vmm-pid>/status`'s non-zero `Seccomp:` mode is retained as the runtime regression guard"* | The **thread-group leader reports `Seccomp: 0` on a correctly-confined CH**; the filters live on `vmm` / `http-server` / `vcpu0` (P5, *the per-thread seccomp correction*) | **The AC fails against correct behaviour.** Must read `/proc/<pid>/task/*/status`. `wave-decisions.md` says this in as many words. |
| **C-6** | Slice 03 US-VM-7 item 3: `setrlimit` on `RLIMIT_FSIZE`, AC *"finite … strictly lower than `overdrive serve`"* — no sizing rule | `RLIMIT_FSIZE` must be `max(rootfs image, guest RAM)` whenever `shared=on`, because `shared=on` backs guest RAM with a memfd and a memfd is a *file* (P5, *the `RLIMIT_FSIZE` × memfd correction*) | Slices 01–03 pass (`shared=on` off); **Slice 04 kills every volume-carrying VM with an opaque `SIGXFSZ`**. Encode the `max` from Slice 01. |
| **C-7** | Slice 02's four variants: kernel **not found**, rootfs not found, hypervisor absent, boot deadline | The measured P1 failure is a kernel that **is** found and is **not loadable** — reported as `VmBoot(UefiLoad(UefiTooBig))`, a firmware size cap that says nothing about image format (P1, `[D5]`) | The vocabulary is missing the one variant the spike explicitly warns about. **Stated precisely:** the failure is not *unhandled* — Slice 02's unclassified-verbatim arm catches it and reports CH's text faithfully. That is accurate reporting of a **misleading upstream message**: the operator reads a firmware size cap and goes looking at file sizes. `[D5]` and `wave-decisions.md` both require *"a format error naming the real problem"*, which the verbatim arm cannot produce. |

**One refinement, not a contradiction.** Slice 02 treats *"no `cloud-hypervisor` on the
host"* as a per-deploy failure. It is a **host** property, not a spec property — it cannot
change between deploys on the same node — so SD-5's boot probe is a strictly better
source for the diagnosis (it can name the paths searched, and it is testable without
breaking the host). Slice 02's ACs are unaffected: the deploy still fails, and the message
improves.

---

### `[REF]` Deferrals — surfaced for user approval, **no issues created**

Per CLAUDE.md, agents must not run `gh issue create`. Each item below needs a user ruling
before any issue exists or any forward-pointer language lands in an artifact.

| # | Deferral | Why out of scope | Recommendation |
|---|---|---|---|
| **D-1** | **Bound the serial dispatch path** — a concurrency limit on `StartAllocation`, and/or a per-action timeout, and/or making `TickContext.deadline` actually enforced (it is constructed and never read) | Control-plane-wide change; every property is pre-existing. SD-3 bounds the blocking inside the driver instead. | **File.** The residual `pending × D` stall is real and this is the only structural fix. |
| **D-2** | **Cross-workload capacity accounting / admission control** — `Resources` on `AllocStatusRow`, real node capacity instead of `baseline_nodes_phase1()`'s 4000 mCPU / 8 GiB, and a construction site for `TransitionReason::NoCapacity` | Pre-existing and structural; a feature of its own. | **File.** VMs make it materially worse (hard vs soft over-admission), so this feature is the moment it becomes visible. |
| **D-3** | **`TransitionReason::OutOfMemory` has no production emit site** — needs a cgroup `memory.events` subscription | Its own mechanism; SD-4's job is to make the limit *correct*, not to diagnose overruns. | **File**, coupled to D-2 — **and rule on it knowing it is in tension with `[D3]`.** A cgroup OOM will surface as `Failed / WorkloadCrashedImmediately { signal: 9 }`, indistinguishable from `kill -9`, with no mention of memory. That is a *misclassified ending* — the class of lie this feature exists to refuse — being knowingly shipped. The deferral may still be right (the mechanism is real work), but it should not be read as routine. |
| **D-4** | **`JournalStore::probe()` is never called in production** — `RedbJournalStore` is handed to `WorkflowEngine::new` (`lib.rs:1820`) with no probe, while the trait doc claims *"Called once at boot"* | Different subsystem entirely; found incidentally while establishing the probe precedent for SD-5. | **File.** An aspirational doc claim against a call site that does not exist is the exact shape `.claude/rules/development.md` § Documentation forbids. |
| **D-5** | **`shmem_enabled=advise` on the appliance image** — host default is `never`, costing ~55% on every `shared=on` path (P11) | Appliance-image / ADR-0068 territory, and only bites once volumes land (Slice 04). | Already flagged not-taken in `wave-decisions.md`. **Confirm it stays flagged**, or file. |
| **D-6** | **P3 — the pinned 6.18 kernel under CH, per shipping arch** | Belongs on CI's LVH kernel-matrix path, not on the metal box. | Already recorded in `wave-decisions.md` as not blocking #42. **No action** unless the user wants it tracked. |

**Two things this wave does NOT hand back as blockers**, because the evidence resolved
them: the uid/gid question (open DESIGN input on Slice 03) is **settled by P5** —
unprivileged uid + `kvm` group against `0660 root:kvm`, no appliance-image change and no
`0666`; and the `--landlock` availability question behind system constraint 7 now has its
answer — a CH floor declared on `--landlock` states exactly what breaks below it, and P5
confirms the flag composes with a real boot at v53.0.

---

### `[REF]` Handoff

**To `nw-ddd-architect` (domain model):**

1. **Restart accounting is a domain rule, not a driver detail.** A platform-initiated reap
   (SD-1) must not consume restart budget — six `serve` restarts against
   `RESTART_BACKOFF_CEILING = 5` would otherwise terminally kill every VM workload on the
   node. Model the distinction between *the workload failed* and *the platform reclaimed
   it*.
2. **The reap is occurrence-bearing.** Per `.claude/rules/development.md` § *"A convergent
   record cannot answer 'did it happen'"* it must reach `LastTerminated` + `restart_count`
   (ADR-0078), not converge silently back to `Running`.
3. **Two `TransitionReason` variants are implied and unowned:** the reap reason (SD-1) and
   the kernel-format reason (C-7). Slice 03's US-VM-7 already claims the "fifth variant"
   slot; these are additional. Variant naming and the "no two share a variant" invariant
   (US-VM-2) are yours.
4. **NEW, from the 2026-08-11 Bar-2 ruling — `Reclamation` now needs an `Action`, and the
   domain model currently says it does not.** A registered `Reconciler` is pure and mutates
   only through the action-shim (ADR-0023), so reclamation needs **≥1 new `Action` variant
   plus its executor surface**. That falsifies the domain wave's *"CREATE NEW: nothing
   structural — no new `Action` variant"* and the `brief.md` sentence *"`ReclaimAllocation`
   in particular maps to **no `Action` at all** — SD-1's boot pass is a converge-on-boot
   pass, not a reconciler action"*. Both need your pass; the system-designer revision did
   **not** edit them. The one Bar-1 claim inside your section that *was* corrected in place
   is the ES/CQRS table's *"Is the reap workflow-shaped?"* cell (the workflow verdict
   stands; the Bar-1 half was replaced by a pointer to SD-1's triage) — flagged here so the
   edit is not a surprise. **Also still reading Bar 1 and left for you:** your DD-6
   cross-check row *"`reconcilers.md` / `workflows.md` triage of SD-1's boot pass —
   independently re-derived and agrees … so it is `reconcilers.md` **Bar 1**"*. The
   workflow half of that re-derivation still stands and is worth keeping; the Bar-1
   conclusion does not.
5. **Also yours, and also mine to hand over rather than fix: the `~2.5 s to full guest RAM
   residency (P13/P14)` citation in your D-3 framing.** That figure is P13's
   `ondemand`-**restore** uffd backfill applied to the cold-boot path, and P5's
   `VmRSS 276,888 kB` at beacon (128 MiB touched) refutes the generalisation. It is
   withdrawn from SD-4 and from this section's estimation block. **It inflated D-3's
   urgency framing** — the claim that survives is *"cgroup OOM becomes the expected overrun
   failure for a VM, and residency trends toward the declared figure over the run at an
   unmeasured rate"*, which still makes D-3 non-routine but does not rest on a
   restore-path number.
6. **The two reclamation regimes are a domain distinction, not an implementation one.** The
   **boot epoch** reclaims every VM allocation with surviving host state (its ending is no
   longer classifiable); a **steady-state tick** reclaims only terminal/unknown allocations
   and stranded artifacts. Both are *Platform Reclamation* in DD-4's vocabulary, both are
   budget-exempt, both are occurrence-bearing — but only the first ends a workload the
   operator believes is running, and the ubiquitous language should not blur them.

   **Superseded by DD-1(b) / § 105a.** *(Marked 2026-08-11, iteration-2 review NEW-5.)*
   The domain wave minted **Artifact Disposal** as a concept distinct from Platform
   Reclamation: the steady-state tick's terminal/unknown-allocation arm
   (`DiscardStrandedArtifacts`) *authors no ending, writes no row, moves neither
   counter* (DD-1(b)'s vocabulary table). "Both are Platform Reclamation, both
   budget-exempt, both occurrence-bearing" is false of that half — an ending is
   exactly what those two predicates are predicates *of*, and Artifact Disposal has
   none to be exempt or occurrence-bearing about. The instinct that boot and
   steady-state are not interchangeable survives: DD-4 still pins them as *"regimes
   of SD-1's reconciler, not Ending Classes"* that must never appear on a row, an
   Ending Class, or a predicate payload. But the distinction did not land as **two
   regimes** — it landed as **one predicate**, `reclamation_authorised`, evaluated
   over observed supervision state (§ 105a.3's `plan_reclamation` table). Boot is
   that predicate's **degenerate case**: the driver's live-handle set reconstructs
   empty at boot (`Observed(∅)`), so the predicate reads `true` for every VM
   allocation by construction — not because a second rule fired.

**To `nw-solution-architect` (application architecture):**

1. **The `Vmm` port must expose a boot-time substrate gate** — SD-5, the sixth trait
   instance of the established `probe()` pattern. **Constraint, not a signature:** it must
   be callable at boot, before any allocation exists, with no per-allocation input, and it
   must be able to refuse. Pinning the receiver and parameter list is yours, not this
   wave's — as is the composition gate that decides whether the adapter is wired at all.
2. **`VmConfig` must carry, as *values*, three things the slices omit:** `image_type=raw`
   per disk (C-2), the per-VM Landlock **directory** grant (C-4), and the
   reserve-adjusted cgroup limit alongside the un-adjusted guest RAM (C-3/SD-4). Making
   these fields rather than call-site strings is what makes C-2 and C-4 unrepresentable
   rather than remembered.
3. **`VmDriver::start` races three outcomes** — beacon ‖ VMM exit ‖ deadline (SD-3). The
   VMM-exit arm carries CH's stderr into `[D5]`'s diagnosis. Pin the signature; per
   CLAUDE.md § *"Implement to the design"* crafters must not improvise it.
4. **Two derivation functions to pin:** `reserve(memory_bytes)` (SD-4 — a **policy
   function evaluated at start time**, never a persisted field, per § *"Persist inputs,
   not derived state"*; the constant is measured in DELIVER, not guessed) and the vCPU
   rounding rule (Slice 05, already open).
5. **Reclamation is Bar 2 (user ruling 2026-08-11) — a registered `Reconciler`, not a
   converge-on-boot pass.** Its boot-epoch drive extends `adopt_on_restart_recovery`'s pass
   and must complete before it (SD-1); two independent passes over the same cgroup tree
   race. **Note the join it needs:** *"is this a VM allocation"* is **not** a field on
   `AllocStatusRow` — `kind` is `WorkloadKind` (ADR-0047), not the driver — so it resolves
   `workload_id` against the intent aggregate and matches `WorkloadDriver::Vm`. Both stores
   are up before the existing boot passes run. **Five things to pin, all yours:**
   (a) the `State`'s host-observation hydration — it must be a **named, separable step
   producing a plain observed-state value**, because that seam is what makes #197's future
   generalisation a refactor rather than a rewrite (SD-1);
   (b) the `View` — field-less per ADR-0079; a "last reclaimed" marker would be the
   fingerprint-as-diff anti-pattern, and here it would gate whether a live VM is killed;
   (c) the steady-state supervision discriminator — hydrated from the driver's live-handle
   set into `actual`, never inferred;
   (d) the boot-epoch drive sharing **one** pure diff and **one** executor set with the
   ticking path — two drivers, one implementation;
   (e) registration **after** the boot passes, and **not** gated on the `Vmm` adapter being
   composed.
   The `plan_vm_reap` / `execute_vm_reap` split survives as a reshape: `reconcile` is the
   pure diff, the Actions are the plan, the executors are the impure half. The
   rmdir-settled-before-adopt obligation binds the **boot-epoch drive only**.
6. **Start-path ordering is load-bearing:** create the per-VM run directory → bind the
   beacon `UnixListener` → create the cgroup scope + limits → clone the rootfs on the
   master's filesystem → spawn CH with the directory grant → race the three outcomes. The
   listener must exist before the guest dials; the clone must not land on tmpfs.
7. **Seven contradictions above (C-1…C-7) are slice-text and AC corrections** in your
   lane. C-5 is the urgent one — an acceptance criterion that **fails against correct
   behaviour**.
8. **Bar-2 fallout inside your `brief.md` section — one site corrected in place, four
   left for you.** The system-designer revision edited only § 108's closing paragraph
   (the Bar-1 claim withdrawn; the plan-value pattern restated as a reshape, with the
   Action-driven executor and the boot-epoch-only settle obligation named). **Deliberately
   NOT touched, and each still reads Bar 1 / boot-pass:** the component-inventory row
   *"`vm_reap` … SD-1's converge-on-boot pass"*; the reliability row *"the boot reap kills
   and re-drives all N"*; the resource row *"swept by the boot pass regardless"* (now false
   in the direction that matters — the sweep is continuous, which is the point of the bar
   change); and the reuse row *"the reclamation row is authored by the boot pass"*. Also
   yours: § 108's `execute_vm_reap` row, whose universe and settle contract now split
   across the boot-epoch drive and the ticking path. In *this* file, your extension table's
   row *"SD-1: the reap is Bar 1, `observe → diff → converge`"* likewise needs the bar
   corrected — the plan-value extension it records survives untouched.
9. **Four new acceptance obligations from the bar change**, for the DISTILL handoff:
   mid-run drift repair without a `serve` restart; a live VMM whose allocation row is
   terminal killed at tick *N*; the boot-epoch pass still settling every `rmdir` before
   `adopt_on_restart_recovery` reads the tree; and — the safety half, easy to omit — a
   **supervised, non-terminal VM surviving every steady-state tick**.
10. **A-3 (`shared=on` measured on x86_64 only) is now a labelled assumption** in the
    estimation block and in `brief.md` § *Cloud Hypervisor VM driver*. Slice 04 designs
    the volume path for both shipping arches on a single-arch measurement; **if `shared=on`
    misbehaves on Arm metal, Slice 04 is x86_64-only until measured.** The volume
    capability gates, not the driver.

**To `nw-platform-architect` / DEVOPS (not this wave, recorded so it is not lost):** the
appliance image must provision the VM data directory on a reflink-capable filesystem and
assert it with a real `FICLONE` probe rather than an fstype string — `infra/metal/provision.sh`
already does exactly this and is the pattern to reuse.

---

## Wave: DESIGN — domain / bounded-context scope (Hera, 2026-08-11)

> **Second of three DESIGN dispatches.** This section covers **domain modelling
> only**: bounded contexts, aggregates, ubiquitous language, and the classification
> rules that bind reconcilers, drivers and the observation surface alike. Failure
> domains, host-state placement, dispatch latency, resource commitment and
> substrate trust are **SD-1 … SD-5** (Titan, above). The `Vmm` trait signature,
> `VmConfig` shape, spec parse surface and driver dispatch are **application
> architecture** and belong to the solution-architect dispatch that follows.
>
> **Scope:** boot a VM through `overdrive serve` + `overdrive deploy`. Slices
> 01–05. Checkpoint/restore, persistent rootfs, warm pools, the chunk store and
> the guest agent's full protocol are #96 / #97 / #100 and are not modelled here.
>
> **SSOT output:** `docs/product/architecture/brief.md` § *Domain Model* →
> *VM workloads — the ending taxonomy, restart accounting, and the driver/kind
> axis*, carrying **DD-1 … DD-6**. The § *System Architecture* section was not
> touched.
>
> **Revision pass, 2026-08-11**, after the adversarial review (Atlas) returned
> NEEDS_REVISION and the system designer revised SD-1. Three items, all recorded
> in place rather than silently edited: **(1)** the Bar-2 ruling falsified this
> wave's *"no new `Action` variant"* — DD-5 now specifies **two**
> (`ReclaimAllocation`, `DiscardStrandedArtifacts`), and the Reuse Analysis
> conclusion is restated below; **(2)** the `~2.5 s to full guest RAM residency
> (P13/P14)` citation in D-3's framing is **withdrawn** as restore-path evidence
> applied to a cold-boot path — **D-3 itself stays open by design**, with its
> urgency deflated to what SD-4's confinement decision earns on its own;
> **(3)** SD-1's two reclamation regimes are ruled **one Ending Class with a
> precondition** plus one non-ending concept (**DD-1(b)**, vocabulary in DD-4).
> Everything the review marked SURVIVING UNCHANGED is untouched.

### `[REF]` Was domain modelling warranted? — **No new context and no new aggregate; yes for exactly one rule**

The dispatch explicitly licensed *"here are the domain rules; there is no new
bounded context and no new aggregate."* **Half of that is the honest answer, and
saying so is the finding — not a way of declining the work.**

**What is NOT here, and would have been manufactured surface.**

| Candidate | Verdict | Why |
|---|---|---|
| A `VM` / `MicroVm` bounded context | **No** | The primary boundary heuristic is language divergence, and there is none: *allocation*, *workload*, *terminal*, *stop*, *restart* mean for a VM exactly what they mean for a process. That identity IS `[G1]` ("one control plane, all workload types"); a context boundary would contradict the feature's own premise. |
| A `VmInstance` aggregate | **No** | It would have precisely the allocation's lifetime and no independent identity — Vernon rule 2's ~70% case where the answer is a value type inside an existing root, not a new root. |
| A new invariant justifying an aggregate | **No** | The only candidate — guest RAM vs cgroup limit (SD-4) — is a **derivation at start time**, not a stored pair. § *"Persist inputs, not derived state"* forbids persisting the reserve, so there is no two-field invariant to protect. Persisting it would have *manufactured* the invariant that then justified the aggregate. |
| ES / CQRS for the VM lifecycle | **No** | Assessed against the full heuristic in `brief.md` § DD-6; every signal reads negative and the audit-trail signal is already answered — boundedly and deliberately — by ADR-0078. |
| A new `AllocState` | **No** | Zero new lifecycle states. Every transition this feature needs already exists; what was missing was the *classification over them*. |

**What IS here — one rule, and it is not VM-specific.** SD-1 introduces the
platform's first **routine, non-exceptional destruction of a healthy running
workload that the platform is then obliged to recreate**. The ending vocabulary
has never needed a word for that because it has never happened. Both available
defaults are wrong, in **opposite** directions, and a third failure sits under
them that SD-1 does not name.

| Ref | Decision | The default that ships if undecided |
|---|---|---|
| **DD-1** | Endings classify into **three** classes; restart eligibility, budget consumption **and** job finalisation are all functions of the class | Reusing `SystemGc` → **every VM stays dead after a `serve` restart**. Reusing nothing → **node-wide `RestartBudgetExhausted` cascade after six restarts**. Fixing only the second → **a reaped Job-kind VM is finalised `Failed{exit_code: 0}`** — a fabricated exit code on a workload that never exited. |
| **DD-2** | The reclamation's durable surface is ADR-0078's `LastTerminated` + `restart_count`, **unchanged**; the exemption applies to the **budget only** | Either the occurrence is erased (ADR-0078's own defect, reproduced in the feature that cites it) or the budget is consumed (DD-1 default 2). One English word — "restart" — covers both counters. |
| **DD-3** | The reason vocabulary has **two axes**; `OutOfMemory`'s missing emit site is a **declared hole**, not a routine deferral | US-VM-2's "no two share a variant" gets applied to a *disposition*, letting a reclamation reason count toward K3's "≥ 4 distinct diagnoses" — K3 satisfied without a fourth diagnosis shipping. |
| **DD-4** | Four terms pinned: Workload Kind vs Workload Driver; Restart Budget vs Restart Count; Platform Reclamation; what `CleanExit` means for a VM | `kind` is read as the driver and SD-1's reap becomes unimplementable; "restart" is zeroed on the wrong counter; `CleanExit` is read as "the platform verified it succeeded". |
| **DD-5** | `Job`'s boundary unchanged; bounded-change contracts pinned per command — **and, after the Bar-2 ruling, two new `Action` variants specified with their payloads constrained** | The budget exemption ships as a comment ("remember to skip the increment") rather than as a complement-equality assertion — the under-declared-universe class this mandate exists to prevent. And, post-ruling: a crafter mints one reclamation `Action` for both jobs, so disposing of a terminal allocation's leftovers overwrites an already-authored ending. |
| **DD-1(b)** | SD-1's **boot epoch** and **steady state** are *regimes*, not Ending Classes: one class (Platform Reclamation) with a precondition, plus one non-ending concept (**Artifact Disposal**) | Regime-named vocabulary that (a) is not derivable from the terminal row, and (b) has no word for node drain / eviction / live migration — all of which reclaim a **live, supervised** instance at **steady state**. |

**The strongest single argument that this was warranted:** DD-1's third default is
**silent and self-consistent**. A reaped Job-kind VM finalised as
`Failed{exit_code: 0}` produces no error, no log, no leak — just a plausible row
carrying a number the workload never returned. It is the same lie
`[D3]` exists to refuse, reached from a direction no slice AC currently looks at,
and it is reachable *only* once SD-1 lands.

---

### `[REF]` DD-1 — Options considered for the third ending class

| | **A. Reuse `StoppedBy::SystemGc`** | **B. Mint nothing — reclamation is a Workload Failure** ← *today's default* | **C. New disposition in `StoppedBy`** ← **recommended** | **D. New `TransitionReason` cause variant** |
|---|---|---|---|---|
| VM restarts after `serve` restart | **No — every VM stays dead** | Yes | Yes | Yes |
| Survives six `serve` restarts | n/a | **No — node-wide `RestartBudgetExhausted`** | Yes | Yes |
| Job-kind VM correctly re-driven, not finalised | n/a | **No — `Failed{0}` fabricated** | Yes, via the `is_natural_exit` clause | Yes, same clause |
| Reads correctly in the existing language | *Wrong* — `SystemGc` means the **intent is gone** | — | **Yes** — `StoppedBy` already *is* the "who ended this" vocabulary | Partly — `TransitionReason`'s cause variants answer "why did it fail", and reclamation is not a failure |
| rkyv cost | none | none | **append-only variant; discipline documented verbatim at `transition_reason.rs:238-253`** | append-only variant |
| Generalises to node drain / eviction / migration | n/a | no | **Yes** | Yes, but invites a per-mechanism cause variant each time |

**C, with the placement stated as a recommendation and the rule stated as
binding.** `StoppedBy` (`transition_reason.rs:229-255`) already carries
`Operator | Reconciler | Process | SystemGc`, is `#[non_exhaustive]`, and
documents append-only discriminants. `PlatformReclaimed` lands one variant away
from its own contrast case: **`SystemGc` = the intent is gone; `PlatformReclaimed`
= the intent stands and the platform owes a replacement.** That is a real domain
distinction, not a shade of one.

**What is binding regardless of placement** (the solution architect owns the
surface): the Ending Class must be derivable from the terminal row alone, the
three classes must be total and disjoint over terminal rows, and no site may
recover the class by matching free text.

**Not VM-shaped, deliberately.** Node drain, eviction under pressure, live
migration and rolling node upgrades are all Platform Reclamation. Minting a
VM-specific word guarantees a parallel model at the first of those — the same
mistake `[D3]` avoided by generalising into system constraint 9 instead of
special-casing `virtiofsd`. **"Reap" is the SD-1 boot pass's implementation name
for one instance of the class; it is not the domain term.**

---

### `[REF]` Reuse Analysis — **hard gate**

Every existing component whose responsibility overlaps this domain model —
**sixteen**, of which rows 15 and 16 were added at review iteration 1 after the
first pass stopped at fourteen. `CREATE NEW` requires evidence that extending is
impossible.

| # | Existing component | Overlap | Verdict | Evidence |
|---|---|---|---|---|
| 1 | `is_intentionally_stopped` (`workload_lifecycle.rs:1096-1111`) | ending classification — "was this deliberate" | **EXTEND (meaning unchanged)** | Already the intentional-stop predicate over `terminal` **or** `reason`. DD-1 requires only that Platform Reclamation does **not** match it. No new predicate; no new call site. |
| 2 | `is_restartable` (`:1116-1120`) | restart eligibility | **EXTEND (meaning unchanged)** | Already `restartable_state && !is_intentionally_stopped`. Reclamation must match it, which it does for free once (1) holds. |
| 3 | `is_natural_exit` (`:1124-1131`) | Job-kind finalisation | **EXTEND** | The **only** predicate that genuinely changes: it must additionally exclude reclamation. Its own docstring already notes it is deliberately a *different, narrower* set than `is_restartable` — so the two are already modelled as distinct questions and the third class slots in without collapsing them. |
| 4 | `StoppedBy` (`transition_reason.rs:229-255`) | "who ended this" vocabulary | **EXTEND** | `#[non_exhaustive]`, four variants, append-only rkyv discriminant discipline stated verbatim at `:238-253` and already exercised twice (`Process` = 2, `SystemGc` = 3). A fifth append is the established move, not a new mechanism. |
| 5 | `TransitionReason` (`:88-210`) | cause vocabulary | **EXTEND** | `#[non_exhaustive]`; Slices 02–04 already mint variants in this shape. C-7's kernel-format variant is one more of the same. |
| 6 | `CrashFacts::advance` + `LastTerminated` + `AllocStatusRow.restart_count` (ADR-0078) | occurrence surface for the reclamation | **REUSE UNCHANGED** | The mechanism already yields the right answer: reap writes terminal → restart writes `Running` at the **same** LWW key (`RestartAllocation` reuses `failed.alloc_id`, `:743-746`) → `advance` (`observation_store.rs:1144-1159`) snapshots and increments. **Changing it to exempt reclamation would erase the occurrence** — ADR-0078's own defect. One docstring clause narrows (see below); the code does not. |
| 7 | `WorkloadLifecycleView.restart_counts` (`:1312`) | restart **budget** | **REUSE UNCHANGED** | Structure and ceiling check (`:678-679`) are correct as they stand. The exemption is at the **increment** site, expressed in DD-5 as a complement-equality assertion. No new View field — a `budget_exempt` flag would be derived state persisted (§ *"Persist inputs, not derived state"*); the class is already on the row. |
| 8 | `Job` aggregate + `WorkloadDriver` (`aggregate/mod.rs:162`) | intent aggregate | **EXTEND** | One variant on a value type the root already owns. `JobEnvelope` V1 → V2 is user-ruled (`[G4]`, intake I-5) and is a schema-evolution event, **not** an aggregate-boundary change. |
| 9 | `WorkloadKind` (ADR-0047) | lifecycle-shape vocabulary | **REUSE UNCHANGED — and explicitly NOT the home for the driver** | `AllocStatusRow.kind ∈ {Job, Service, Schedule}`. Encoding "is this a VM" here would fuse two orthogonal axes (every `Kind × Driver` pair is meaningful) and break ADR-0047's discriminator. The join is intent-side: `workload_id → Job → WorkloadDriver::Vm`. |
| 10 | `AllocState` (enum at `observation_store.rs:198`; `is_terminal()` at `:221-224`) | lifecycle bucket | **REUSE UNCHANGED** | Zero new states. The feature needs a classification *over* terminal states, not a new terminal state. A `Reclaimed` state would force every existing `is_terminal()` / match site to change and would put the disposition in the wrong type. |
| 11 | `ExitKind` + `exit_observer::classify` + `WorkloadLifecycle` restart/backoff | exit classification, restart | **REUSE UNCHANGED** | Slice 03's stated learning hypothesis, and this model does nothing to disturb it. Reclamation is produced by the **boot pass**, not by the exit observer, so it never reaches `classify`. (The adjacent `intentional_stop` surface is discharged separately at row 15 — `classify` is not the whole of that seam.) |
| 12 | `TerminalCondition` (`transition_reason.rs:407`; `Completed` at `:442`, `Failed` at `:460`) | reconciler terminal claim | **REUSE UNCHANGED** | `is_intentionally_stopped` reads it, so the classification must not collide; but reclamation needs no `TerminalCondition` of its own, because it is precisely the case where **no terminal claim should be made** — the run is not over (DD-1). `Completed` / `Failed` are the two variants DD-1 trap 3 turns on. |
| 13 | `TransitionSource::Driver(DriverType)` (`api.rs:710`, variant at `:714`) | who reported a transition | **REUSE UNCHANGED** | The enum derives `Serialize, Deserialize, ToSchema` and **no rkyv** (`api.rs:707`), so it is serde/utoipa only (intake I-5). This is the evidence for DD-4's *"`DriverType` is not persisted on any row"* pin. `MicroVm`'s deletion is a variant removal on this path and nothing more. |
| 14 | ADR-0078 § *unreachable in Phase 1* clause (`observation_store.rs:1122-1132`) | reachability claim about `Terminated → Running` | **EXTEND (documentation)** | Reclamation makes that transition reachable for the first time, and reachable **correctly**. The clause's advice still stands for *operator* stops (excluded upstream by `is_restartable`). Must be amended in the same commit per § *Documentation* and the behaviour-change-marks-stale-adjacent-docs discipline — **not** left to contradict the code. |
| 15 | `ExitEvent.intentional_stop` (`traits/driver.rs:299-303`, contract `:278-283`, ordering invariant `:293-298`) | **the existing two-class ending discriminator**, one layer below DD-1's predicates | **REUSE UNCHANGED** | In-tree this flag is *"the load-bearing discriminator that distinguishes operator-driven termination … from natural crashes"* — i.e. today's binary classifier, and the obvious place a crafter would try to put a third class. It **cannot** carry one (it is a `bool`), and it **cannot accidentally claim the reclamation**: after a `serve` restart `ExecDriver.live` is reconstructed empty (SD-1), so no watcher holds the flag and **no `ExitEvent` is produced for a surviving VMM at all** — the reap authors its terminal row directly. Row 11's `exit_observer::classify` argument does not by itself discharge this; the empty-`live` fact does. |
| 16 | `ServiceLifecycle` — `ServiceAllocFact.restart_count` (`service_lifecycle.rs:194-205`), ceiling check (`:778`), its own `Action::RestartAllocation` (`:798`) and its **five** action-emitting sites (`:568`, `:593`, `:631`, `:779`, `:983`) | **a second reconciler over the same restart budget, the same ceiling, and its own terminal claims** | **EXTEND** *(corrected at review iteration 2 — see below)* | **Budget half: clear.** Its docstring (`:196-205`) states it composes with — does not duplicate — `WorkloadLifecycleView::restart_counts` per ADR-0055 §7: it **consumes** the budget as a hydrated input and never increments it (`restart_counts` has exactly one writer workspace-wide, `workload_lifecycle.rs:788`), so DD-1's exemption is automatically consistent here. **Terminal-claim half: NOT clear, and the first pass got this wrong.** It certified the component from the liveness branch alone — the one branch that *is* state-gated (`:769`, `state == Running`, unreachable for a reclaimed alloc). The enclosing loop at `:500` filters **no** state, and `startup_probe_failed_action` (`:968-991`, emitted `:651-658`) gates on `started_at.is_some()` ∧ attempts ∧ deadline ∧ no-Pass with **no `AllocState` gate at all** — so a Service alloc reclaimed after Running but before Stable receives a fabricated `ServiceFailed { StartupProbeFailed }`. **That is DD-1 trap 3's shape on the Service path**, and it is why DD-1's rule is now stated in its general form (*no reconciler may author a terminal claim on a Platform-Reclamation row*) rather than as a list of `WorkloadLifecycle` predicates. |

**CREATE NEW — restated 2026-08-11 after the Bar-2 ruling. The ruling changed the
answer, and the change is recorded rather than edited away.**

**What the first pass concluded:** *"nothing structural — no new aggregate, no new
context, no new type, no new state, no new predicate, no new store, no new View
field, and no new `Action` variant."* **The last clause is now false.**

**What changed, and it was not a modelling error.** The conclusion was drawn
against SD-1 as it then stood — a **converge-on-boot pass**, which invokes an
executor directly, so the reclamation effect never had to cross the publication
boundary and genuinely added no domain surface. The user's 2026-08-11 ruling makes
reclamation a registered **`Reconciler`** (`reconcilers.md` Bar 2). A registrant on
the reconciler runtime is a pure function that mutates **only** through `Action`s
dispatched by the action-shim (ADR-0023). So the effect must now cross that
boundary **as data** — and the `Action` variant DD-5's naming note forbade a
crafter from minting is exactly the surface the design now has to specify. *(That
naming note is itself rewritten in `brief.md` § DD-5; the prohibition inverts —
mint precisely what is specified, improvise nothing.)*

**The new structural delta, in full:**

| New | Why it cannot be an extension | Contract |
|---|---|---|
| **`Action::ReclaimAllocation { alloc_id }`** | No existing `Action` authors a terminal row for an allocation whose instance the platform is destroying. `StopAllocation` is an Intentional Stop by construction (`Driver::stop` sets `intentional_stop`), which is DD-1's default 1 — the one that leaves every VM dead after a `serve` restart | `brief.md` § DD-5, unchanged declared delta; payload is `alloc_id` and nothing else — **no disposition parameter, no regime field** |
| **`Action::DiscardStrandedArtifacts { alloc_id }`** | Its declared delta over the observation universe is **empty**, which no existing `Action` has. Folding it into the row above would author an ending for an allocation whose ending is already authored — clobbering `LastTerminated`, incrementing `restart_count` for a restart that never happens, and re-labelling an operator's Intentional Stop as a platform one | `brief.md` §§ DD-1(b), DD-5; complement equality is the whole row plus `restart_counts[alloc_id]` |
| Their two **executor** surfaces on the action-shim | An `Action` with no executor is not dispatchable | Shape is the solution architect's; the two payload prohibitions are not |

**What survives the restatement, and it is most of it.** Every one of the sixteen
reuse verdicts above stands unchanged — the ruling moved the *effect surface*, not
the *vocabulary*. There is still no new aggregate, no new bounded context, no new
`AllocState`, no new store, no new View field and no new predicate; the domain
vocabulary delta is still **new variants on two existing `#[non_exhaustive]`
enums plus one clause on one existing predicate**. What is no longer true is the
stronger claim that the wave adds *no structure at all*. The honest verdict is
**minimal domain modelling with two new `Action` variants forced by an upstream bar
change** — demonstrated by the table above rather than asserted in prose, which is
the same standard the original conclusion was held to.

---

### `[REF]` Contradiction check against SD-1 … SD-5 and against the spike

**No contradictions.** Three items where this model **sharpens** an upstream
statement, and one where it **extends** it.

| # | Upstream | This model |
|---|---|---|
| 1 | SD-1: *"the reap's terminal row carries a distinct reason and is excluded from the budget"* | **Sharpened, and one consequence added.** "Distinct reason" is necessary and **not sufficient**: a merely-distinct reason still satisfies `is_natural_exit` (`:1124-1131`), so for a **Job**-kind VM the finalise branch at `:622-624` fires *before* the restart branch at `:673` and `classify_natural_exit_terminal` (`:1136-1146`) falls through to `TerminalCondition::Failed { exit_code: Some(0) }`. The VM is finalised as a failed job with a fabricated exit code and never restarted. **SD-1 does not name this; DD-1 does.** SD-1's decision is unaffected — the reclamation is still mandatory. *(Its **bar** is not: revised 2026-08-11 to Bar 2 per the user ruling. The trap above is bar-independent — it fires on the terminal row's classification, not on what drove the pass that wrote it.)* |
| 2 | SD-1: *"it must reach the durable `LastTerminated` snapshot per ADR-0078"* | **Confirmed, mechanism identified, and a hazard closed.** No change to `CrashFacts::advance` is needed or wanted; DD-2 states this explicitly so nobody "fixes" `advance` to exempt reclamation and thereby erases the occurrence SD-1 asked to preserve. |
| 3 | SD-1: *"is this a VM allocation is a two-surface join, not a row field"* | **Confirmed and promoted to a language pin (DD-4).** The failure mode is a *vocabulary* defect — "kind" colloquially covers both axes — so the durable fix is a glossary entry, not only a code comment on one boot pass. |
| 4 | SD-4 / deferral **D-3**: *`TransitionReason::OutOfMemory` has no production emit site* | **Modelled, not resolved — and its exposure is shown to grow.** SD-4 correctly confines an overrun to the allocation's own scope, which makes **cgroup OOM the *expected* VM overrun failure** rather than a rare one: a VM's declared RAM is a standing claim whose host-resident share trends toward the declared figure over the run and does not shrink back, **at a rate unmeasured on the cold-boot path**; a process typically makes no such claim at all. DD-3 records it as a **declared hole** with a one-sentence discharge condition. No resolution is proposed and none is implied. *(Revised 2026-08-11: this cell previously read "a VM reaches full residency in ~2.5 s, P13/P14". That is P13's `ondemand`-**restore** uffd backfill applied to the cold-boot path, refuted by the design's own P5 (`VmRSS 276,888 kB` at beacon, 128 MiB touched). **Withdrawn — it inflated D-3's urgency framing.** D-3 stays open by design, and the claim above is what keeps it non-routine without leaning on a restore-path number.)* |
| — | `reconcilers.md` / `workflows.md` triage of SD-1's reclamation | **Half stands, half is withdrawn — revised 2026-08-11.** **Stands:** the workflow disqualification. `workflows.md` criterion 3 fails (every step idempotent, every partial-crash state converging on the next pass — verbatim the *end-to-end-idempotent fire-and-forget* non-candidate), so reclamation is **not** a workflow. **Withdrawn:** the Bar-1 conclusion and the `veth_provisioner::provision` analogy. Per the user ruling, it is `reconcilers.md` **Bar 2** — see § *System Architecture* → SD-1 for the triage. **And the manner of the error is worth recording:** this cross-check reported *"independently re-derived and agrees"*, but it agreed because it ran **the same test the upstream draft ran** — the workflow-disqualification — and then inherited the upstream's Bar-1 conclusion without running the Bar-1-vs-Bar-2 question (*does `actual` drift while the system is up?*). **Two derivations that share a missing step corroborate nothing**; a cross-check must name which tests it performed, or its agreement is an echo. |
| — | The spike (P1, P2, P4, P5) | **No conflict.** P2 is the evidence for DD-4's `CleanExit` pin and, via the one-connection-carries-both property, for why SD-1 must reap rather than adopt. P1/P5's misleading upstream errors are the evidence for DD-6's ACL classification and for C-7 being an ACL leak. Nothing in the model depends on P6–P14 (banked for #96 / #97 / #100). |

---

### `[REF]` Deferrals and open items — surfaced for user approval, **no issues created**

Per CLAUDE.md, agents must not run `gh issue create`. Each item needs a user
ruling before any issue exists or any forward-pointer language lands.

| # | Item | Why out of scope here | Recommendation |
|---|---|---|---|
| **H-1** | **DD-1 is ADR-worthy and no ADR was minted.** The Ending Class rule changes a **platform-wide** classification (it governs node drain, eviction and migration, none of which are VM-specific) and narrows an accepted clause in ADR-0078 | Minting an ADR that amends ADR-0078 and ADR-0037 while deferral **D-3** is still unruled risks landing a contradiction. Recorded in `brief.md` § *Domain Model* instead — uniform with SD-1…SD-5, which this same wave recorded there without an ADR | **User ruling requested.** If approved, the next free number is **ADR-0081** (0080 is the highest in tree). Recommend approving: the rule outlives this feature. **Strengthened by the 2026-08-11 revision** — DD-1(b) adds a platform-wide *authorisation precondition* (*a reconciler may author a Platform Reclamation exactly when the platform can no longer honestly classify that instance's ending*) that governs node drain, eviction under pressure and live migration, none of which are VM-specific and none of which this feature ships. A rule that constrains three unbuilt subsystems is ADR-shaped by any reading. *(Numbering note: ADR-0082 and ADR-0083 have since been minted by the application-architecture wave, so the next free number moves accordingly — confirm at mint time rather than copying 0081 from here.)* |
| **H-2** | **The `StoppedBy` vs `TransitionReason` placement for the reclamation disposition** | Enum surface is the solution architect's lane; DD-1's *rule* is binding regardless | **No user action.** Recommendation (`StoppedBy::PlatformReclaimed`) with rationale is in `brief.md` § DD-1; the solution architect pins it. |
| **H-3** | **`ADR-0031:539` names a `[microvm]` TOML table that intake I-5 deleted** | An accepted ADR carrying a table name that will not exist. Editing an ADR goes through the architect agent, and this one is outside #42's stated scope | **Surface for ruling.** Cheapest correct fix is a one-line amendment noting `[vm]` supersedes `[microvm]`. Not done here. |
| **H-4** | **`TransitionReason::NoCapacity` is an *undeclared* hole AND a false doc claim** (deferral **D-2**'s vocabulary half) | The *mechanism* (admission control) is a feature of its own and stays in D-2. The *doc claim* is not a deferral at all | **Split the item.** (a) **Fix now, in-scope:** the emit-inventory at `transition_reason.rs:55` marks `NoCapacity` emitted **`yes`** while it has no production construction site — a false documentation claim of exactly the shape `.claude/rules/development.md` § *Documentation* forbids, and the same violation DD-2 already obliges to be fixed for `advance`'s clause. Correct it to `NO` in the same commit. *(Contrast `OutOfMemory` at `:56`, which correctly declares itself `NO — Phase 2`.)* (b) **Fold into D-2's ruling:** building the emit site. **Corrected at review iteration 1** — the first pass called this "the second declared hole", which understated it. |

**Two things this wave does NOT hand back as blockers.** The aggregate question is
closed — `Job` is unchanged in boundary, so no ruling is required on aggregate
design; and the `TransitionReason` **axis** question is closed by DD-3, so no
ruling is required on which invariant applies where.

**One assigned item is deliberately re-routed rather than delivered, and it is
named as such.** Titan's handoff item 3 assigned *"variant naming"* to this wave.
This wave delivers the naming for the **Disposition** axis
(`StoppedBy::PlatformReclaimed`, DD-1) and the **meaning** of C-7's Cause variant
(*kernel image format not loadable by this hypervisor*, DD-3) — because both are
domain-language decisions. It does **not** name the individual Slice 02 / 03 / 04
Cause variants: those are one-to-one with driver failure modes whose surface the
solution architect is pinning in the same wave, and naming them here would pin an
enum shape from outside the lane that owns it. **Re-assigned to
`nw-solution-architect`, bounded by DD-3's two-axis rule** (Cause variants only,
US-VM-2 / K3 apply to them, dispositions must not be counted). This is a lane
call, not a gap — flagged so nobody reads item 3 as silently dropped.

---

### `[REF]` Handoff

**To `nw-solution-architect` (application architecture — runs last):**

1. **DD-1's rule is general — *no reconciler may author a terminal claim on a
   Platform-Reclamation row* — and it binds sites in TWO reconcilers, neither of
   which is in SD-1's handoff.**
   - `WorkloadLifecycle`: beyond `is_intentionally_stopped` / `is_restartable` /
     the budget increment (`:788`, the only writer workspace-wide), the
     **`is_natural_exit` clause** (`workload_lifecycle.rs:1124-1131`) must exclude
     Platform Reclamation, or a reaped **Job**-kind VM is finalised
     `TerminalCondition::Failed { exit_code: Some(0) }` at `:622-624` and never
     restarted. **The finalise branch runs before the restart branch, so fixing
     the budget alone does not reach it.**
   - `ServiceLifecycle`: `startup_probe_failed_action`
     (`service_lifecycle.rs:968-991`, emitted `:651-658`) has **no `AllocState`
     gate**, and the enclosing loop at `:500` filters no state — a Service alloc
     reclaimed after Running but before Stable gets a fabricated
     `ServiceFailed { StartupProbeFailed }`. Its liveness branch (`:769`) is
     already safe; the other four emit sites are not covered by that gate.
2. **The reclaimed `AllocState` is constrained, not free.** `Failed` is excluded
   on domain grounds (it asserts a run ended that did not end) **and** because it
   makes `service_lifecycle.rs:611`'s EarlyExit branch reachable, fabricating an
   `exit_code` for a workload that never exited. `Terminated` is indicated; any
   other candidate must be checked against *"does a reconciler failure branch key
   on this state?"* first. (`brief.md` § DD-1, boundary note.)
3. **Pin the disposition's surface.** Recommendation is
   `StoppedBy::PlatformReclaimed` (appended; rationale in `brief.md` § DD-1).
   Binding regardless of your choice: the Ending Class is derivable from the
   terminal row alone, the three classes are total and disjoint, and no site
   recovers the class from free text. Per CLAUDE.md § *"Implement to the design"*,
   pin the exact variant and forbid crafters inventing a parallel flag.
4. **The budget exemption ships as a complement-equality assertion, not a
   comment.** `brief.md` § DD-5 gives the declared delta and complement for
   `ReclaimAllocation` and `RestartAfterReclamation`;
   `restart_counts[alloc_id]` sits **outside** the restart's declared delta.
   Carry that shape into the roadmap ACs so the exemption cannot be
   under-declared.
5. **Do NOT change `CrashFacts::advance`.** It already produces the right answer
   for reclamation. **Three doc corrections ride in the same commits**, all of
   them the behaviour-change-marks-stale-adjacent-docs discipline rather than new
   work: (a) `advance`'s *"unreachable in Phase 1"* clause
   (`observation_store.rs:1122-1132`) — reclamation makes `Terminated → Running`
   on the same key reachable, and correctly so; (b) the emit-inventory at
   `transition_reason.rs:55`, which marks `NoCapacity` emitted **`yes`** against
   no production construction site; (c)
   `crates/overdrive-core/src/aggregate/mod.rs:166`
   (`// Future Phase 2+: MicroVm(MicroVm), Wasm(Wasm).`) — the comment sits inside
   the enum you are adding `Vm` to, and your commit is what makes it false.
6. **C-7's variant is a *Cause*; the reclamation disposition is not.** Keep
   US-VM-2's "no two share a variant" and K3's "≥ 4 distinct" scoped to the
   **Cause** axis (`brief.md` § DD-3). A disposition counted toward K3 would let
   the KPI pass without a fourth diagnosis shipping. **Naming the individual
   Slice 02 / 03 / 04 Cause variants is re-assigned to you** (see § Deferrals):
   this wave pins the Disposition name and C-7's *meaning*, and leaves the Cause
   variants to the lane pinning the driver's failure surface — bounded by DD-3's
   two-axis rule.
7. **`AllocationSpec` (ADR-0030) needs a driver discriminator; the persisted
   observation row does not.** Keep `WorkloadKind` free of driver information —
   it is the lifecycle axis (ADR-0047), and the two axes are orthogonal
   (`brief.md` § DD-4).
8. **Language, at every operator-facing surface:** `vm`, never `microvm`; and no
   artifact may state or imply that a VM's reported exit status is independently
   verified by the platform — `CleanExit` means *the guest agent reported a clean
   exit* (`brief.md` § DD-4).
9. **NEW, from the 2026-08-11 Bar-2 ruling — the reclamation reconciler emits
   TWO `Action` variants, not one, and the split is a domain rule.** This wave's
   *"no new `Action` variant"* is withdrawn (see § *Reuse Analysis*). Specified in
   `brief.md` § DD-5; yours to place, name-check and give executors:
   - **`Action::ReclaimAllocation { alloc_id }`** — authorises a **Platform
     Reclamation**: destroys the surviving host state **and** authors the terminal
     row. Emitted only when the allocation is **non-terminal** *and* the platform
     holds **no live supervision handle** for it.
   - **`Action::DiscardStrandedArtifacts { alloc_id }`** — **Artifact Disposal**:
     removes host state for a **terminal or unknown** allocation and **authors
     nothing**. Its declared delta over the observation universe is **empty**;
     the complement is the *entire* row plus `restart_counts[alloc_id]`.
   - **Do not collapse them into one variant with a flag.** A
     `{ alloc_id, authors_ending: bool }` puts the Ending Class in a
     caller-declared boolean — the mistake DD-2 rejects for
     `ExitEvent.intentional_stop`, and a sentinel where a sum type belongs. A
     single collapsed variant re-labels an operator's Intentional Stop as a
     platform one on the unstoppable-orphan path (SD-1), clobbers
     `LastTerminated`, and increments `restart_count` for a restart that never
     happens.
   - **Two payload prohibitions are binding:** no disposition parameter
     (`StoppedBy::PlatformReclaimed` is *constant* for the first variant — a `by:`
     field re-opens DD-1 default 1 from inside the Action), and **no regime
     field** (`boot_epoch` / `is_boot` / `steady_state`): the safety check keys on
     the observed live-handle set in `actual`, never on a self-declared flag, and
     the Ending Class must stay derivable from the terminal row alone.
   - Both are keyed on `AllocationId` **and nothing else** — the executor
     re-observes, so the Action names *which* ensemble to converge, never what was
     found. For an **unknown** allocation the key comes from the artifact itself,
     which is why SD-1's clone-filename attribution is load-bearing rather than
     stylistic.
   Per CLAUDE.md § *"Implement to the design"*: pin these exactly and forbid a
   crafter minting a third variant, a flag, or an extra payload field.
10. **`brief.md` § DD-1(b) is the taxonomy ruling behind item 9**, and it binds
    your reconciler design: *a reconciler may author a Platform Reclamation
    exactly when the platform can no longer honestly classify that instance's
    ending.* The boot epoch is the **degenerate case** of that predicate (the
    live-handle set is reconstructed empty), not a second rule — which is the
    domain justification for SD-1's *"one pure diff, two drivers"*. **Boot epoch**
    and **steady state** are regimes; they must not appear in a row, an Ending
    Class, an `Action` payload or a predicate name.

**To `nw-acceptance-designer` (DISTILL):**

1. **Four scenarios that do not exist in any slice today**, all reachable only
   once SD-1 lands: (a) a reaped **Job**-kind VM is **re-driven, not finalised** —
   the `Failed{exit_code: 0}` fabrication; (b) **six consecutive `serve` restarts
   leave every VM running** and `RestartBudgetExhausted` is never reached; (c)
   after a reclaim-and-restart, `workload describe` shows `restart_count`
   incremented **and** `last_terminated` populated with the reclamation
   disposition; (d) **the Service-path analogue of (a)** — an alloc reclaimed
   after Running but before Stable is not handed a
   `ServiceFailed { StartupProbeFailed }` for probes that never failed
   (`service_lifecycle.rs:968-991`). Note (d) is only reachable for `[vm]` +
   `[service]` specs, which Slice 02 rejects at deploy time — so it is a
   **`WorkloadKind::Service` + exec** case today and a VM case only once #257
   lands. Cover it anyway: the reclamation class is not VM-specific.
2. **The budget/count pair is one scenario, not two.** Asserting only the budget
   passes a implementation that erased the occurrence; asserting only the count
   passes one that consumed the budget. Assert both in the same case.
3. **A fifth scenario, NEW from the 2026-08-11 Bar-2 ruling — Artifact Disposal
   authors nothing.** A live VMM whose allocation row is **already terminal** is
   killed at steady-state tick *N*, and the `alloc_status` row is **byte-unchanged
   afterwards**: `restart_count` does not move, `last_terminated` is not
   overwritten, `state` / `terminal` / `reason` / `updated_at` are untouched. This
   is the domain half of the review's *"a live VMM whose allocation row is terminal
   killed at tick N"* AC — killing the VMM is the system-level half; **not writing
   a row is the domain half, and only this assertion catches it.** The failure it
   defends against is concrete: the terminal row may record an operator's
   Intentional Stop whose kill failed (SD-1's unstoppable-orphan case), and a
   disposal that re-writes it re-labels the operator's own stop as a platform
   reclamation, then credits a restart that never happens. (`brief.md` §§ DD-1(b),
   DD-5 — its declared delta over the observation universe is empty, so the
   assertion is a whole-universe `after == before`.)
4. **Mutation targets.** Collapsing Platform Reclamation into either neighbouring
   class must be killed — into Intentional Stop by scenario (a)/(b), into Workload
   Failure by (b)/(c). **And, post-ruling:** collapsing the two reclamation
   `Action`s into one must be killed by scenario 3 — a single variant that always
   authors a row fails its whole-universe complement equality.

**To the user, for ruling:** **H-1** (mint an ADR for DD-1 + DD-1(b)? — next free
number to be confirmed at mint time, since ADR-0082/0083 have landed since this
was written), **H-3** (ADR-0031:539's stale `[microvm]` table), **H-4(b)** (fold
`NoCapacity`'s emit site into D-2's scope — note **H-4(a)**, the false `yes` in the
emit inventory, is an in-scope fix and needs no ruling). And **D-3 is unresolved by
design** — `brief.md` § DD-3 states the cost of shipping it open, which is that a
cgroup OOM ships as `signal: 9` on the workload class most likely to cause one.
*(Revised 2026-08-11: D-3's framing no longer cites the withdrawn `~2.5 s`
residency figure. The cost is unchanged and the urgency is now carried by SD-4's
own confinement decision — cgroup OOM becomes the **expected** VM overrun failure —
rather than by a restore-path number. **The deferral itself is still open and still
wants a ruling.**)*

**Nothing in the 2026-08-11 revision is handed back as a new blocker.** The two
`Action` variants are in-scope structure, not a deferral; the one surface they
depend on that does not exist today — an observable supervision-handle set — is
already assigned to the solution architect by SD-1's handoff item 5(c), and
DD-1(b)'s fail-safe clause states what the reconciler must do while it is
unavailable (nothing).

---

## Wave: DESIGN — application / component scope (Morgan, 2026-08-11)

> **Third and last of three DESIGN dispatches.** This section covers
> **application-level** architecture: the `Vmm` port surface, the `VmConfig`
> value, driver dispatch, the spec-parse surface, the reason vocabulary, and the
> wiring that lets `overdrive deploy` actually reach a hypervisor. Failure
> domains, host-state placement, dispatch latency, resource commitment and
> substrate trust are **SD-1 … SD-5** (Titan). Bounded contexts, aggregates,
> ubiquitous language and the ending taxonomy are **DD-1 … DD-6** (Hera).
> **Neither section is amended; both are consumed.**
>
> **Scope, per the user's 2026-08-10 ruling:** boot a VM through
> `overdrive serve` + `overdrive deploy`. Slices 01–05.
>
> **SSOT output:** `docs/product/architecture/brief.md` § *Application
> Architecture* → *Cloud Hypervisor VM driver extension* (§ 99–114), plus
> **ADR-0082** (the `Vmm` port and the `VmConfig` anti-corruption value) and
> **ADR-0083** (`DriverRegistry`, the per-driver `AllocationSpec` payload, the
> composition gate, and the DD-1 binding).

### `[REF]` Was application design warranted? — **Yes, and this is the wave with the most to build**

The dispatch licensed *"little is needed here"* and noted it was unlikely. It is
not the honest answer, and the reason is specific rather than volumetric:
**every one of the three preceding waves' decisions terminates in a code surface
that does not exist yet, and three of those surfaces are load-bearing for the
feature's own pass/fail bar.**

A repo-wide search confirms the starting point: outside `docs/` and
`spike-scratch/`, there is **no Rust code for this feature at all**. One comment
in `overdrive-sim/src/adapters/driver.rs:8` (*"without spawning a real VMM"*)
and two pre-existing `DriverType` variants (`MicroVm` at `traits/driver.rs:43`,
`Vm` at `:45`) are the entire footprint. No `Vmm`, no `VmConfig`, no
`cloud-hypervisor` crate dependency, nothing in any `Cargo.toml`.

**Three gaps, each of which ships a broken feature if undecided:**

| Gap | Current state | Default that ships if undecided |
|---|---|---|
| **The composition root composes exactly one driver** (`compose_production_driver` declared `lib.rs:1401`, composing at `:1422-1425`, into `AppState.driver: Arc<dyn Driver>`) | one hardcoded `ExecDriver` | **A complete VM mechanism no production path can reach** — verbatim intake precedent warning #1, where the reference implementation's `create_virtualizer()` has zero callers and `OPENCAPSULE_VMM=cloud-hypervisor` changes nothing. This is `[G1]` and it is the feature's pass/fail bar |
| **`AllocationSpec` has no driver discriminator** (`traits/driver.rs:139-142`: flat `command`/`args`) | the shim reads `driver.r#type()` *from the driver it already holds* (`action_shim/mod.rs:1301`) | Circular the moment there are two drivers — a registry would have nothing to key on |
| **The parser has no driver-table dispatch at all** | `workload_spec.rs:710` hardcodes `contains_key("exec")` → `MissingExec` → `parse_section(table, "exec")`; there is **no `DriverInput` in that parser** (the one at `aggregate/mod.rs:906` feeds the *legacy* path) | *"Exactly one driver table"* has no representation to be exactly-one **of** |

**And a fourth that is silent.** Hera's DD-1 binds sites in two reconcilers, and
neither is in Titan's handoff. The Job-kind trap fabricates
`TerminalCondition::Failed { exit_code: Some(0) }` on a workload that never
exited; the Service-path trap fabricates `ServiceFailed { StartupProbeFailed }`
for probes that never failed. Both produce no error, no log and no leak — just a
plausible row.

**What is NOT here, and would have been over-engineering.** No new
architectural style. No new bounded context or aggregate (Hera's finding,
consumed). **No change to the `Driver` trait** — intake I-2 explicitly licensed
changing it (*"if the `Driver` trait needs to change to accommodate it, change
it"*), and after building the design it turns out not to need changing:
`VmDriver` provides the five required methods and takes all five defaults. That
licence going unexercised is a finding, not an omission. No new third-party
dependency. No HTTP client for CH's API socket, because no path in this feature
depends on it.

### `[REF]` Estimation — the numbers this design rests on

Every figure is measured (spike, CH v53.0, bare metal x86_64) or read off a
cited code fact. Two quantities remain **unmeasured and are labelled as such**;
both were inherited from Titan and both go to DELIVER.

```
── inherited, load-bearing, MEASURED ─────────────────────────────────────
guest reaches /init         0.730 – 0.746 s   12/12 runs, 16 ms spread   (P1)
ready beacon at host        ~1.1 s                                       (P2)
same, nested aarch64        ~8.7 s                                       (P2)
reflink clone, 4 GiB        0.015 s / +0 MiB  vs 3.970 s / +4096 MiB     (P4)
VMM RSS above a 2 GiB guest ~5.4 MiB steady / ~11.9 MiB pre-residency    (P13)

── this wave's derived constants ─────────────────────────────────────────
VM_BOOT_DEADLINE            30 s     = 8.7 s (worst measured substrate)
                                       × ~3.4 margin, covering guest fsck
                                       + three CONFIG_VSOCKETS=m module loads
                                     ⇒ ~300 ticks at DEFAULT_TICK_CADENCE
reserve_bytes(declared)     UNKNOWN  RED scaffold. Floors bracket it at
                                     ~5.4–11.9 MiB, and BOTH are RSS-derived
                                     while the cgroup also charges page tables
                                     that RSS cannot see. DELIVER measures via
                                     memory.current / memory.stat.

── inherited, UNMEASURED, labelled ───────────────────────────────────────
CH failure-to-exit latency  assumed ≪ VM_BOOT_DEADLINE. If wrong, SD-3
                            option C (async readiness seam) re-opens.
ExecDriver::start absolute  unused in any decision here.
```

**`reserve_bytes` is the single hardest DELIVER dependency this design creates**,
and it is deliberately a RED scaffold rather than a guess. Shipping a constant
between two RSS-derived floors, when the quantity being bounded is charged
partly outside RSS, is intake precedent warning #7's "magic version floor"
failure with different units.

**Blast radius of the `AllocationSpec` change**, since it is the widest
mechanical edit: ten irrefutable `let WorkloadDriver::Exec(..) =`
destructures become `match`es (ADR-0031's deliberate tripwires, `:197`); every
`AllocationSpec` construction site moves to `driver: DriverPayload`, including
`workload_lifecycle.rs:743-776`'s `RestartAllocation` spec. All
compiler-enforced; none silent. `AllocationSpec` derives neither serde nor rkyv
(`traits/driver.rs:132`), so **no envelope bump** — in contrast to
`WorkloadDriver::Vm`, which does trigger `JobEnvelope` V1 → V2 (user-ruled at
intake I-5).

### `[REF]` The decisions — options considered and trade-offs named

Full option analysis is in the two ADRs; the four decisions where a different
choice was genuinely available are summarised here.

#### Driver dispatch — three options

| | **A. `DriverRegistry`** ← **recommended** | **B. A two-arm `match` in the shim** | **C. A `Driver` facade that dispatches internally** |
|---|---|---|---|
| Pre-committed by ADR-0022 | **yes, naming this exact trigger** | no | no |
| Where the driver set lives | composition root, as data | one shim function | inside a driver |
| Expresses SD-5's capability gate | **as a missing map entry** | needs a `bool`/`Option` *beside* the match — a second representation of the same fact that can disagree with it | hides it inside a driver that is always composed |
| Honest capability logging | `drivers.kinds()` | ad hoc | ad hoc |
| Cost | one new value type + `AppState` field change | smallest | a driver that is not a driver |

**A**, and the deciding reason is the third row. SD-5 requires *"CH absent ⇒ node
boots, `[vm]` rejected at admission"* and *"CH present + lying substrate ⇒ refuse
to boot"*. With a registry the first case is `!drivers.supports(DriverType::Vm)`.
With B it is a second fact that can drift from the match.

#### The composition gate — three options

| | **A. Compose `VmDriver` unconditionally, refuse at `start`** | **B. `[node] drivers = [...]` operator knob** | **C. Discover the binary, probe, insert** ← **recommended** |
|---|---|---|---|
| Precedent | **none** — this is SD-5 option B, withdrawn after its evidence was found inverted | none; DISCUSS scoped no node-capability config | **already ships** — `MtlsEnforcement::probe` / `MtlsResolve::probe` inside `if compose_mtls` |
| Failure disposition | a misconfiguration hides as *"every VM deploy fails at runtime"* | a knob can disagree with reality | substrate **lie** → refuse the node; capability **absence** → not a fault |
| Cost | buries the lie exactly as the substrate buries it | an unstated knob (CLAUDE.md: out of scope by default) | the inverse hazard below |

**C.** Its honest cost, restated because it is a real operational surprise:
**installing the `cloud-hypervisor` package can flip a node from booting to
refusing to boot** if its staging filesystem cannot reflink. That is correct
behaviour — a node advertising VM support on a substrate that cannot honestly
deliver it should not run — and it lands at the next `serve` boot, not at install
time.

#### Graceful shutdown — two options

| | **A. CH API `PUT /api/v1/vm.power-button`** | **B. `SHUTDOWN` on the guest's open vsock connection** ← **recommended** |
|---|---|---|
| Works with a ~200-line PID 1 | **no** — no `acpid`, so on x86_64 the ACPI button event has no in-guest consumer | yes |
| Works on aarch64 | **no** — CH uses PSCI, not an ACPI button | yes, identically |
| New transport | an HTTP-over-unix client dependency | **none** — the guest already holds the connection from `READY` until `EXIT n` |
| Evidence | not probed | **transport and lifetime proven (P2); the host→guest command byte is unprobed** — see below |

**Corrected at review iteration 3 (2026-08-11).** This row previously read
*"Proven: P2, both arches"*, which was an **evidence overclaim**. P2 exercised
the vsock connection **guest→host only** (`spike/findings.md:357`; the host→guest
direction is recorded as explicitly not established at `:2787`), and no probe had
a guest agent *read* its socket while supervising a child. The `SHUTDOWN` command
byte is a **host→guest write on the accepted connection**, so the mechanism's
first real evidence is the **Slice-03 Tier-3 stop AC**, not the spike. **The
decision stands unchanged**: the two facts that reject option A (no `acpid` in a
~200-line PID 1; aarch64 uses PSCI rather than an ACPI button) are independent of
this mechanism, and `VM_SHUTDOWN_REQUEST_DEADLINE` (2 s, ADR-0082 § D4) bounds
the failure to extra latency on a path that lands
`Terminated / Stopped { by: Operator }` either way.

**B**, with `--api-socket` nevertheless kept in `VmConfig` — one socket in an
already-granted directory, its Landlock rule auto-derived by CH, and the
substrate `Driver::resize` will need for GH #92. **No path in this feature
depends on it**, and that is stated so nobody assumes otherwise.

#### The Ending Class surface — two options

| | **A. An `EndingClass` enum unifying the three predicates** | **B. One new predicate + a totality proptest** ← **recommended** |
|---|---|---|
| Totality / disjointness | **structural** | tested |
| Blast radius | a refactor of a working classifier across two reconcilers + the CLI renderer | three lines |
| Hera's reuse gate | contradicts *"no new type"* | consistent |

**B**, with A recorded as the shape to reach for if a **fourth** class ever
appears. The trade is explicit: DD-1's *"total and disjoint over terminal rows"*
becomes a property test rather than a compile-time guarantee, which is why the
proptest is mandatory rather than nice-to-have.

### `[REF]` Reuse Analysis — **hard gate**

The full **37-row** table is in `brief.md` § 112 and is not duplicated. Summary:
**37 existing components assessed** — 13 of Titan's system-scope rows re-checked
at the application scope, 12 in the first pass, 6 added at review iteration 1,
1 at iteration 2, and **5 added at iteration 3** when the Bar-2 ruling landed
(`Reconciler` + the four dispatch enums, the field-less-View precedent,
`CgroupFs`'s write-only contract, the Earned-Trust probe family, the `Invariant`
catalogue).

**Two verdict REVERSALS at iteration 3, recorded rather than edited over.**
Row 14 (`Driver` trait) moves **REUSE UNCHANGED → EXTEND**: the first two passes
recorded intake I-2's licence to change the trait as *deliberately unexercised*,
and that was correct **for Bar 1**; a Bar-2 reconciler needs the supervision
discriminator read from the component that holds the handle, so the trait gains
one **defaulted, sync** `live_allocations(&self) -> Option<Vec<AllocationId>>`
whose `None` default is the fail-safe reading. Row 31 (`spawn_convergence_loop`)
moves **REUSE → EXTEND** for the sweep submission. Both reversals have one cause:
a Bar-1 pass invokes an executor directly and needs neither a wake nor a
supervision discriminator.

**The gate failed on its first pass, and the failure is recorded rather than
quietly patched.** The table specified `DriverRegistry` for the
`StartAllocation` path only, while **three other seams consume the single
`AppState.driver`** the design replaces: `exit_observer::spawn_with_runtime`
(one task, one `take_exit_receiver`, one captured `driver_kind`), the shim's
stop/terminal arms (whose `Action` variants carry **no spec and no
`workload_id`**), and `MtlsInterceptWorker::start_alloc` (fail-closed, with a
docstring predicate a second driver falsifies). Left unfixed, the design would
have shipped a VM that starts, cannot be stopped, whose exit never reaches the
ObservationStore — killing `[D3]`, the feature's north star, on the production
path — and which gets host-socket mTLS interception installed on a datapath its
guest traffic never traverses. A hard gate that misses the consumers of the
field being replaced is the gate failing, not a detail.

**CREATE NEW — seven items, all pre-ratified before this wave** *(six until
iteration 3; the Bar-2 ruling split `vm_reap` into the `VmReclamation` reconciler
and added the `VmHostState` port)*:

| New | Why extending is impossible | Pre-ratified by |
|---|---|---|
| `Vmm` port + the `VmConfig` value family | Every host effect (spawn, `FICLONE`, vsock UDS, Landlock) is unreachable from Tier-1 DST without a port, **and** Slice 03's fail-closed AC requires injection *at a port boundary* because the whole test envelope runs on one Lima kernel. No existing port has hypervisor semantics | intake **I-2** |
| `CloudHypervisorVmm` + `SimVmm` | The two halves of that port | intake I-2 |
| `VmDriver` | `ExecDriver` is exec-shaped throughout (`Child`, `WEXITSTATUS`, `send_sigkill_pgrp`) | intake I-2, ADR-0029 |
| `DriverRegistry` | A second driver cannot be reached without changing `lib.rs:1422-1425` | **ADR-0022's pre-committed migration** |
| `VmReclamation` reconciler + `plan_reclamation` + `ReclaimAllocation` / `DiscardStrandedArtifacts` + their two executors | SD-1's own new surface, at Bar 2. Every half extends an existing pattern (reuse rows 1, 3, 33, 34), and **DD-5 specifies the two variants**, so minting them is implementing the design rather than inventing surface | SD-1 (user ruling), **DD-5** |
| `VmHostState` port + `RealVmHostState` + `SimVmHostState` | `CgroupFs` is deliberately **write-only** (`traits/cgroup_fs.rs:58-257`; its `write` postcondition is phrased against a *hypothetical* read) and two of the three observation surfaces are not cgroupfs at all. Without a port the reconciler's `actual` is unreachable from Tier-1 DST, and `observe()` is the **named separable seam** SD-1's pin 1 makes a design obligation | SD-1 pin 1 |
| `overdrive-init` crate | There is no in-guest code today | `[D4]`, Slice 01 |

**Zero new third-party dependencies.** `nix` (already an `overdrive-worker`
dependency) supplies `FICLONE` / `setns` / `setrlimit`; `tokio` supplies the
process and `UnixListener` surfaces.

**One honest note on what the dst-lint gate would NOT have caught.**
`xtask/src/dst_lint.rs`'s `BANNED_APIS` covers
`tokio::net::{TcpStream, TcpListener, UdpSocket}` but **not** `Command` and
**not** `UnixListener`, and the `std::fs`-in-`async` rule scans `adapter-host`
only. So the lint alone would not have blocked a portless VM driver. The `Vmm`
port is required by `.claude/rules/testing.md` § *"Nondeterminism must be
injectable"* and by Slice 03's AC, **not** by the current lint — and intake I-2's
second justification (*"those same calls are banned on any `core`-class compile
path"*) is only true of the calls that are actually in `BANNED_APIS`. Stated so
the port's justification rests on the rule that genuinely forces it. Widening
`BANNED_APIS` is **not** proposed here (control-plane-wide scope creep) but is
noted as a real gap.

### `[REF]` Contradiction check — against SD-1…SD-5, DD-1…DD-6, the spike, and the slices

**No contradictions with Titan or Hera.** Rows 1–8 were the first pass's five
sharpenings and two extensions plus one lane call; **rows 9–11 were added
2026-08-11** when the user's Bar-2 ruling landed upstream. Row 10 is the one
place this lane delivers an upstream **property** by a **different mechanism**
than the one named, and it is written as such rather than left for a reader to
notice. **Row 10 is CLOSED as of 2026-08-11** — SD-1 pin 5 was revised to assert
the property and to name registration as inert, so the two sections agree and
cross-reference each other; the substitution is recorded, not outstanding. **Row
11's `VM_RECLAMATION_SWEEP_INTERVAL = 30 s` was ratified by the user on the same
date**, mechanism and value both.

| # | Upstream | This wave |
|---|---|---|
| 1 | SD-5: *"how the composition root expresses that gate is a solution-architect decision"* | **Answered.** The gate is a missing `DriverRegistry` key. Absence and admission-rejection become the same fact, so they cannot disagree |
| 2 | SD-3: *"`VmDriver::start` races three outcomes"* | **Sharpened with `biased;`** — Titan named the three arms but not the tie-break. Beacon wins: a guest that beaconed and then died is a *started* VM whose ending belongs to the exit watcher, not to `start`. Also added: **every non-`Ok` arm cleans up before returning, including the deadline arm** — the arm an implementation is most likely to leak on |
| 3 | SD-1: *"the boot reap extends `adopt_on_restart_recovery`'s pass and must run before it"* | **Sharpened on placement.** `adopt_on_restart_recovery` sits **inside `if state.mtls_worker.is_some()`** (`lib.rs:2131`). The reap must run **outside** that gate — VM allocations exist whether or not mTLS is composed — so it goes immediately *before* the `if`, which satisfies the ordering in both branches. Titan's constraint is preserved, not weakened |
| 4 | SD-1 (**revised 2026-08-11**): reclamation is **Bar 2 — a registered `Reconciler`**, with five pin obligations | **Delivered in `brief.md` § 105a and ADR-0083 § D7**, and the plan-value pattern **survives as a reshape, not a loss**: `plan_reclamation` is the **pure** diff and takes **no port** at all, the two `Action`s **are** the plan, and the two executors are the impure half. The bug class *"the observe pass wrote something"* stays non-representable — the diff has nothing to write with. All five pins discharged: **(1)** `VmHostState::observe()` is the named, separable hydration seam returning a plain `VmHostObservation`; **(2)** `VmReclamationView` is **field-less** (ADR-0079); **(3)** the supervision discriminator is an observed input on `actual`, read through a new defaulted `Driver::live_allocations`, with `SupervisionSet::Unavailable` as the **`Default`** so an unpopulated half authorises nothing; **(4)** one diff, one executor pair, two drivers; **(5)** registration unconditional — see sharpening 9 |
| 5 | SD-5 + SD-1: reclamation and the probe | **Sharpened: reclamation is NOT `Vmm`-gated**, and the mechanism is now explicit. It reaches the host through the new **`VmHostState`** port, composed **unconditionally** like `CgroupFs`, never through `Vmm` — and with no `Vm` registry entry the supervision set is `Observed(∅)`, so an uninstalled-CH node's survivors are *authorised* rather than stranded. `VmHostState::probe()` asks a **different** question from `Vmm::probe`'s scenario 5 (*enumerable* versus *creatable-and-bindable*), and an **absent** root is `Ok` there — a node that never ran a VM must still boot. This is also why artifact *removal* is deliberately **not** a `Vmm` method (ADR-0082 § A6) |
| 6 | DD-1: *"no reconciler may author a terminal claim on a Platform-Reclamation row"*, with the shape of each change left to me | **Bound at five sites across two reconcilers** (`is_natural_exit`; the View writes at `:788-799`; **the backoff-ceiling branch at `:679`/`:703`**; `startup_probe_failed_action`; and the four `ServiceLifecycle` branches checked and found already state-gated). `is_intentionally_stopped` and `is_restartable` need **no** change. **Enforced by an emission-level property (P2), not by the site list** — the ceiling branch was missed by the first draft's own three-line claim, which is the argument for P2 |
| 7 | DD-5's declared universe (`alloc_status[alloc_id]` ∪ `restart_counts[alloc_id]`) | **Extended by one slot: `last_failure_seen_at[alloc_id]`, declared complement-equal.** It is failure memory, and a reclamation is not a failure — and stamping it would make a reclaimed workload serve a backoff window before returning, the opposite of SD-1's stated intent. An extension, not a contradiction |
| 8 | DD-3 / Hera's re-assignment: *"naming the individual Slice 02/03/04 Cause variants is re-assigned to you, bounded by DD-3's two-axis rule"* | **Delivered: twelve `TransitionReason::Vm*` variants** (ADR-0083 § D5). All on the **Cause** axis; the reclamation **disposition** is deliberately excluded and must not be counted toward K3, which twelve exceeds three times over anyway |
| 9 | DD-1(b) + DD-5 (**added 2026-08-11**): two `Action` variants, two payload prohibitions, regimes never in a row / Ending Class / payload / predicate | **Adopted verbatim, names included.** `Action::ReclaimAllocation { alloc_id }` and `Action::DiscardStrandedArtifacts { alloc_id }`, appended after `LivenessExhausted` (`reconcilers/mod.rs:615`; `Action` derives neither serde nor rkyv, so **no envelope bump**). No disposition parameter, no regime field, no artifact enumeration — the executor re-observes. **The boot epoch is not a case anywhere in the design**: at boot the enumeration returns `Observed(∅)` and the one predicate is true for every VM allocation by construction, which is Hera's *"degenerate case of the steady-state rule rather than a second rule"* as code rather than as prose. `execute_discard_stranded_artifacts` has **no `ObservationStore` and no broker parameter**, so DD-5's *"declared delta empty over the observation universe"* is structural rather than remembered |
| 10 | SD-1 pin 5: *"the reconciler is registered after the boot sequence completes"* | **Property delivered; mechanism sharpened, and stated rather than quietly diverged.** Registration must sit at the existing site (`lib.rs:1525-1773`) because `register` takes `&mut self` and `Arc::new(runtime)` at `:1774` precedes `AppState`'s construction, which the boot passes at `:2131-2147` then read. But registration is **inert** — it probes the `ViewStore` and `bulk_load`s views and drives no tick — and the only production driver of ticks, `spawn_convergence_loop`, is spawned at `lib.rs:2314-2320`, **strictly after** the boot passes. **That spawn ordering is the load-bearing fact** and is pinned as such (brief § 105a.7). SD-1's second half — *"not gated on `Vmm` composition"* — holds verbatim. **CLOSED 2026-08-11** — SD-1 pin 5 now asserts the property, names registration **inert**, pins the strictly-after spawn as the constraint, and its C4 L2 registration edge reads the same; the cross-reference is mutual and no divergence remains. The `&mut self` / `Arc::new(runtime)` reason is retained deliberately — it is why registration order is not the constraint |
| 11 | SD-1's *"steady-state ticks"* | **Extended with the mechanism they need, because Bar 2 does not supply it.** `EvaluationBroker` is purely event-driven, there is no bootstrap sweep, and `has_work` only *re*-enqueues a reconciler that already ticked — so **nothing in tree would ever tick `vm-reclamation`**. `spawn_convergence_loop` submits one evaluation every **`VM_RECLAMATION_SWEEP_INTERVAL = 30 s`** on its already-injected `Clock` (DST-controllable), and the boot drive submits one on completion. Three alternatives rejected, including a `last_swept_at` **View field** — which SD-1's pin 2 forbids, and which the runtime fsyncs *before* dispatching so it would record "last attempted". This is the **one** place the design touches shared convergence machinery, named as such. **RATIFIED by the user 2026-08-11** — mechanism *and* value; the constant is compile-time, **not operator-tunable, and no knob is promised** (a property, not an open question). The derivation and the three rejected alternatives are retained in brief § 105a.8, which is the single site for them |
| — | The spike (P1, P2, P4, P5) | **No conflict.** P5's uid answer is consumed as-is (Slice 03's "open DESIGN input" is **answered**, not returned as a blocker); P1's arch split parameterises `KernelImage::validate`; P2 is the evidence for both the beacon protocol and the shutdown mechanism; P4's intra-filesystem constraint fixes where the clone lands |

**Two apparent divergences that are lane calls, stated so they are not read as
contradictions:**

1. **`ParseError::MissingExec` is deleted rather than kept alongside.** Intake
   I-5's single-cut ruling and § "Deletion discipline" both apply; there is no
   alias and no grace period.
2. **`Vmm` carries no guest-facing surface**, so a reader looking for "how does
   the platform talk to the guest" will not find it on the hypervisor port. It
   is on `VmDriver`, which owns the beacon session. Intended boundary, not an
   omission — and the constraint that forced it (`CloudHypervisorVmm` lives in
   `overdrive-host` and cannot reach a connection `overdrive-worker` accepted)
   is what corrected the first draft's unimplementable `Vmm::shutdown`.

*(A third item — keeping a three-variant `SeccompMode` so a mutation site would
exist — was withdrawn at review iteration 1. The reasoning was wrong: the
**renderer** is a mutation site regardless of the enum's cardinality, so
`VmConfinement::seccomp_arg()` satisfies Slice 01's AC **and** makes `Off`/`Log`
unrepresentable. It was the one place the design abandoned its own governing
rule; it no longer does.)*

**Two slice corrections beyond Titan's C-1…C-7**, both consequences of decisions
made here:

- **Slice 03's *"open DESIGN input: which uid/gid the hypervisor drops to"* is
  closed, not returned.** P5 settled it — an unprivileged uid in the `kvm` group
  against `0660 root:kvm` — with no appliance-image change, which is exactly the
  constraint the slice attached to the question.
- **Slice 04's *"open DESIGN input: whether `[[vm.volume]]` reaches the persisted
  `Job` aggregate"* is answered: it does.** `VmPayload.volumes` is part of
  `WorkloadDriver::Vm`, so it rides inside **Slice 01's `JobEnvelope` V2** and
  needs no second bump. The slice asks for this to be settled *before* it is
  built, not inside it — it is settled here.

### `[REF]` Deferrals and open items — surfaced for user approval, **no issues created**

Per CLAUDE.md, agents must not run `gh issue create`. Titan's **D-1 … D-6** and
Hera's **H-1 … H-4** stand unchanged and are not restated. New from this wave:

| # | Item | Why out of scope here | Recommendation |
|---|---|---|---|
| **M-1** | **`reserve_bytes` ships as a RED scaffold and blocks VM memory limits.** SD-4's reserve has two RSS-derived floors and no measured boot-path value, and the cgroup charges page tables RSS cannot see | Not a deferral of *scope* — it is a **measurement DELIVER must take** (`memory.current` / `memory.stat` against a real boot). Recorded so it is not discovered mid-slice | **No issue.** It is in-scope DELIVER work with a named method. Flagged because a crafter who guesses here ships intake-precedent-#7 |
| **M-2** | **`xtask dst_lint`'s `BANNED_APIS` does not cover `Command` or `UnixListener`**, and the `std::fs`-in-`async` rule scans `adapter-host` only — so the lint would not have blocked a portless VM driver | Widening the banned set is a **control-plane-wide** change touching every crate class; doing it inside a driver feature is scope creep | **Surface for ruling.** Recommend filing: the gap is real and the `Vmm` port currently rests on a rule (DST injectability) rather than on the gate that is supposed to enforce it |
| **M-3** | **`ADR-0022` is partially superseded by ADR-0083** (its single `AppState.driver` decision). Its deferral *reasoning* is preserved and unamended | Editing an accepted ADR's Status line goes through the architect agent; ADR-0083 records the supersession from its own side | **No user action** unless the user wants ADR-0022's Status line amended to `Superseded in part by ADR-0083` — recommended for index hygiene |
| **M-4** | **ADR-0081 is left unallocated**, reserved for Hera's **H-1** (promote DD-1 to an ADR), which is a pending user ruling. This wave took **0082** and **0083**. *Re-checked 2026-08-11: `adr-0080`, `adr-0082` and `adr-0083` exist on disk and `adr-0081` does not, so the reservation still holds* | Minting 0081 here would either collide with H-1 or pre-empt a user ruling | **No action** — but whoever mints H-1 must **confirm the next free number at mint time** by listing `docs/product/architecture/adr-*.md` rather than inheriting "0081" from this note or from Hera's (now older) reference. A reserved number is a claim about a directory, and directories change |
| **M-5** | **`overdrive-init` adds two static musl build targets** (`{x86_64,aarch64}-unknown-linux-musl`) to the toolchain and CI, and the platform must publish it as an artifact operators bake into a BYO rootfs | The *build/publish pipeline* is DEVOPS-wave work (`nw-platform-architect`), not application architecture. The crate, its class and its protocol are decided here | **Surface for ruling** on the publication mechanism only. The crate itself is in-scope Slice 01 work per `[D4]` |

**Two things this wave does NOT hand back as blockers**, because the evidence or
the design closed them: Slice 03's uid/gid open input (**answered by P5**) and
Slice 04's `[[vm.volume]]`-in-the-aggregate open input (**answered: it rides in
Slice 01's V2 bump**).

**One thing that is not a deferral and must not be read as one.** Slice 04's
**lift trigger** (*"if this slice exceeds 9 days, lifting it into its own feature
is pre-authorised… DESIGN owns the re-run at the DESIGN handoff and returns a
blocker if it fires"*). Re-run here: this design **reduces** Slice 04's estimate
rather than increasing it — `VmConfig`'s volume payload, the derived `shared=on`
flag, the `[[vm.volume]]` parse surface and the four failure variants are all
shaped by decisions already made for Slices 01–03, and the conditional second
rkyv bump is **eliminated** (volumes ride Slice 01's V2). **The trigger does not
fire; no blocker is returned.**

### `[REF]` Handoff

**To `nw-acceptance-designer` (DISTILL):**

1. **Pinned signatures crafters must not improvise** (CLAUDE.md
   § *"Implement to the design"*): the `Vmm` trait's four methods (note
   `terminate`, **not** `shutdown` — the guest request is `VmDriver::stop`'s);
   the `tokio::select!` three-way race including `biased;` and
   `VmExitWatch::recv(&mut self)`; `MemoryPlan::derive` as the **only**
   `MemoryPlan` constructor; and `action_shim::dispatch(drivers:
   &DriverRegistry, …)`. **Added 2026-08-11 with the Bar-2 ruling** (brief
   § 105a, ADR-0083 § D7): the `VmHostState` trait's four methods;
   `plan_reclamation(desired, actual) -> Vec<Action>` as a **pure free function
   taking no port**; `SupervisionSet` with **exactly two** inhabitants and
   `Unavailable` as its `Default`; `SupervisionSet::reclamation_authorised`;
   `Driver::live_allocations(&self) -> Option<Vec<AllocationId>>` **defaulted and
   sync**; and the two `Action` variants with `alloc_id`-only payloads. An
   implementation that adds a second `MemoryPlan` constructor, a fifth `Vmm`
   method, an `image_type` field, a `SeccompMode` enum, **a third
   `SupervisionSet` variant, a `boot_epoch` field on either `Action`, a single
   `ReclaimAllocation { authors_ending: bool }`, or any `View` field on
   `VmReclamationView`** has **diverged**, even if every test passes.
2. **Six scenarios that exist in no slice today.** (a) `Vmm::probe` refuses the
   node on a non-reflink staging directory, and the node **boots normally** when
   `cloud-hypervisor` is merely absent — two cases, one disposition each, and
   conflating them is the failure SD-5 exists to prevent. (b) A `[vm]` deploy on
   a node with no VM driver is rejected **at admission**, naming the absent
   capability — not a parse error. (c) The **deadline** arm of the race leaks
   nothing: no VMM process, no clone, no run directory, no cgroup scope. (d) A
   guest reports `EXIT 7` and the VMM then exits `0` **in that order** — the
   report is not overwritten by teardown. (e) Both `[exec]` and `[vm]` present
   ⇒ `MultipleDriverSections`; neither ⇒ `MissingDriverSection`. (f) A reclaimed
   **Service** alloc (Hera's scenario (d)) — note it is an exec-Service case
   today, since `[vm]` + `[service]` is rejected until #257.
2a. **Five MORE scenarios from the Bar-2 ruling** (brief § 105a.10 — full text
   there; summarised so DISTILL does not have to infer them). **(g) Mid-run drift
   repair WITHOUT a `serve` restart** — stop an allocation, make its scope
   removal fail, and a later tick reclaims the stranded scope, run directory and
   clone. *This is the AC that distinguishes Bar 2 from Bar 1; under
   converge-on-boot it can only pass by restarting the process.* **(h)** A live
   VMM whose allocation row is terminal is killed at tick *N*. **(i)** The
   boot-epoch drive's `rmdir`s are **settled before `adopt_on_restart_recovery`
   reads** — with N surviving VM scopes, adopt does not refuse the boot, and every
   reclaimed allocation's netns is treated as orphaned and reclaimed. **(j) THE
   SAFETY HALF — a supervised, non-terminal VM SURVIVES every tick**: process,
   scope, run directory, clone and row all still present after repeated sweeps.
   *Without (j) the reconciler passes its entire suite by killing everything.*
   **(k)** A live VMM whose row is **already terminal** is killed at tick *N*
   **and the row is BYTE-UNCHANGED afterwards** — every field including
   `updated_at`, `last_terminated`, `restart_count`, plus
   `restart_counts[alloc_id]`. **Only (k) catches an implementation that
   collapsed the two `Action`s into one**: a collapsed implementation still kills
   the VMM and still passes (h). Plus **(l)** a node that uninstalled
   `cloud-hypervisor` still reclaims its survivors.
3. **C-5 is an AC correction, not a new scenario.** Slice 01's seccomp runtime
   guard must read `/proc/<pid>/task/*/status`; `/proc/<pid>/status` reports
   `Seccomp: 0` on a **correctly** confined CH. The argv assertion stays — CH's
   `log` mode installs a filter and would survive a `/proc`-only check.
4. **Mandatory mutation targets** (≥ 80 % kill): `MemoryPlan::derive`,
   `KernelImage::validate`, `DiskAttachment::to_disk_arg`,
   `VmConfinement::seccomp_arg`, `VmConfig::rlimit_fsize`, **`plan_reclamation`**,
   **`SupervisionSet::reclamation_authorised`**, each
   of the three race arms, the `[D3]` classification join, `is_natural_exit`'s
   reclamation clause, the budget-exemption guard, **the backoff-ceiling
   reclamation guard**, `startup_probe_failed_action`'s reclamation guard, and
   the parser's driver-table dispatch. **`reserve_bytes` joins this list at the
   DELIVER step that gives it a body**, not at Slice 01 — a `todo!()` has
   nothing to mutate, so listing it earlier is a vacuously satisfiable gate.
5. **Tests that are the design's own enforcement**, not coverage:
   `vmm_equivalence.rs` and **`vm_host_state_equivalence.rs`** (both adapters,
   one contract each — the latter is where `kill_scope`'s **settle**
   postcondition is asserted); the Ending-Class totality/disjointness proptest
   **P1**; the emission-level property **P2**, which now ranges over **three**
   reconcilers because `VmReclamation` enters its scope; **P3** (the disposal
   path leaves the row byte-identical — DD-5's degenerate complement equality);
   and **P4** (`Unavailable` authorises nothing, for every input).
6. **ESR specifications** for the new reconciler, in the tree's mechanical form —
   an `Invariant` variant plus an async evaluator, **not** an `assert_always!`
   macro (those are prose classes in this repo). Three:
   `VmReclamationConverges` (eventually), `SupervisedVmSurvivesEveryTick`
   (always — AC (j) as an invariant), `VmReclamationIdempotentSteadyState`
   (always). `ReconcilerIsPure` is reused unchanged. DST reachability is
   `SimVmHostState` driven with a `SimClock`, so the 30 s sweep cadence is
   **advanced deterministically rather than waited on**.

**To `nw-software-crafter` (DELIVER):**

1. **Measure `reserve_bytes` before writing it** — `memory.current` /
   `memory.stat` against a real boot, **not** RSS. Cite the measurement in the
   function's docstring. A guessed constant is a rejection.
2. **Measure CH's failure-to-exit latency** while you are there. It is SD-3's
   only unmeasured premise, and if it approaches `VM_BOOT_DEADLINE` the design
   says to re-open SD-3 option C rather than paper over it.
3. **Measure `cp --reflink=auto` cross-device** to close Titan's C-1 inference
   (P4 measured `auto` succeeding on capable XFS and `always` failing
   cross-device; the silent-full-copy degradation is stated counterfactually).
   The design does not depend on it — it uses `FICLONE` directly — but the
   inference should be closed rather than inherited.
4. **Five documentation corrections land in the commits that make them false**,
   not later: `transition_reason.rs:55`'s `NoCapacity` emit claim (**H-4(a)**),
   the two variants missing from that inventory, `CrashFacts::advance`'s
   *"unreachable in Phase 1"* clause, and **two** stale forward pointers on two
   distinct types — `aggregate/mod.rs:166`'s `MicroVm(MicroVm)` inside
   `WorkloadDriver` and `aggregate/mod.rs:909`'s `MicroVm(MicroVmInput)` inside
   `DriverInput`.

**To `nw-platform-architect` / DEVOPS (not this wave, recorded so it is not
lost):** the appliance image must provision the VM data directory on a
**reflink-capable** filesystem and assert it with a real `FICLONE` probe rather
than an fstype string — `infra/metal/provision.sh:419-430` already does exactly
this. Keep `infra/provision/common-system.sh:73-76`'s hard-fail on a CH build
without `--landlock`; it is the version floor, stated against a capability.
`overdrive-init` adds two static musl targets to CI (**M-5**). Tier-3 gating for
VM boot **must not** run on the nested-Apple Lima path: a green run there is
evidence, a red run is uninformative, and the two are indistinguishable.

**External integrations — one, and contract testing does not apply.** Cloud
Hypervisor is a **local child process** over a CLI argument surface and a UNIX
socket, not a network service: there is no provider to verify against and no
versioned wire contract. The analogous risk — an upstream whose behaviour
changes under us — is answered by `Vmm::probe()`'s capability assertions at boot,
a version floor named against a **capability**, and the `vmm_equivalence`
contract test. That combination *is* the consumer-driven contract for a local
binary.

---

## Changelog

| Date | Wave | Change |
|---|---|---|
| 2026-08-11 | DESIGN (application — revision after cross-wave adversarial review, Morgan) | Applied this lane's four items from the adversarial review (fable, `NEEDS_REVISION`) plus the cascade from Titan's and Hera's revisions. **R-C1 (CRITICAL, cascaded) — reclamation is reshaped into a registered `Reconciler` (Bar 2), per user ruling.** `plan_vm_reap` / `execute_vm_reap` / `VmReapPlan` are **deleted**; the plan-value split survives as `plan_reclamation` (**pure, and it takes no port at all**, so *"the observe pass wrote something"* stays non-representable) plus **two** executors. New `brief.md` § **105a** (eleven sub-sections) is the pinned shape; ADR-0083 § D7 is rewritten and gains **A7–A9**. **Titan's five pins, all discharged:** (1) the hydration seam is a **named port method** — `VmHostState::observe() -> VmHostObservation`, one call, one plain value, so #197's generalisation is a refactor rather than a rewrite; (2) `VmReclamationView` is **field-less** (ADR-0079), with retry falling out of the runtime's `has_work` re-enqueue rather than a marker the runtime fsyncs *before* dispatching; (3) the supervision discriminator is an **observed input on `actual`**, read through a new **defaulted, sync** `Driver::live_allocations(&self) -> Option<Vec<AllocationId>>`, and it **fails safe by construction** — `SupervisionSet`'s `Default` is `Unavailable`, which authorises nothing, so reading the unpopulated half of the state degrades to "do nothing this tick" rather than "kill a live VM"; (4) **one observation, one pure diff, one executor pair, two drivers** — the boot drive calls the executors directly rather than through `dispatch_single` only because that function takes fifteen parameters none of which reclamation touches; (5) registration is **unconditional** (never `Vmm`-gated), and **the "no tick interleaves" property is delivered by the convergence loop's spawn point (`lib.rs:2314-2320`), not by registration order** — `register` takes `&mut self` and `Arc::new(runtime)` at `:1774` precedes `AppState`, so "register last" is structurally unavailable; registration is inert and the spawn ordering is the load-bearing fact. **Hera's DD-1(b)/DD-5 adopted verbatim, names included:** two `Action` variants (`ReclaimAllocation` authors an ending, `DiscardStrandedArtifacts` authors none), `alloc_id`-only payloads, **no disposition parameter and no regime field**, and **no `boot_epoch` anywhere in the design** — at boot the enumeration returns `Observed(∅)` and the single predicate is true for every VM allocation by construction. `execute_discard_stranded_artifacts` has **no `ObservationStore` and no broker parameter**, making DD-5's empty declared delta structural rather than remembered. **One gap the ruling created and did not fill, found here:** the broker is purely event-driven with no bootstrap sweep and `has_work` only *re*-enqueues, so **nothing in tree would ever tick `vm-reclamation`** — `spawn_convergence_loop` gains one submission every `VM_RECLAMATION_SWEEP_INTERVAL = 30 s` on its injected `Clock` (DST-controllable), with a `last_swept_at` View field, unconditional self-re-enqueue, and event-only wakes all rejected with reasons. **One precision SD-1's prose leaves implicit:** the three surfaces are **not equally attributable** — the cgroup tree is shared with exec allocations, so a scope with no row is left alone (which is what preserves `ExecDriver`'s survive-a-restart behaviour), while the run directory and the clone are VM-exclusive by construction, so **a scope is never the sole trigger**. Reuse table **31 → 37 rows with two verdict REVERSALS recorded rather than edited over** (row 14 `Driver` REUSE→EXTEND; row 31 `spawn_convergence_loop` REUSE→EXTEND), and a `VmHostState` port justified against `CgroupFs`'s deliberately write-only contract. Five new ACs specified (mid-run drift repair without a `serve` restart; terminal-row VMM killed at tick N; boot `rmdir` settled before adopt; **the safety half — a supervised non-terminal VM survives every tick**; and Hera's row-**byte-unchanged** assertion, the only one that catches a collapsed single-`Action` implementation), plus P2 extended to three reconcilers, new properties **P3**/**P4**, and three `Invariant` variants for ESR. **R-H2 (HIGH) — evidence overclaim corrected**: the graceful-shutdown row said *"Proven: P2, both arches"*; P2 exercised the vsock connection **guest→host only** (`findings.md:357`, host→guest recorded as not established at `:2787`) and no probe wrote host→guest or had a guest agent read while supervising a child. Relabelled *"transport and lifetime proven (P2); the host→guest command byte is unprobed — first exercised by the Slice-03 Tier-3 stop AC"*; **the decision stands** (no `acpid`, aarch64 PSCI, and the 2 s escalation bounds the failure). **R-H4 (HIGH) — C-1…C-7 landed in the slices**, which DISTILL reads: `superseded-by-DESIGN` markers naming the governing ADR/brief section on slices 01–05, plus in-place corrections — **C-1** `cp --reflink=auto` → the `FICLONE` ioctl (slice-01, slice-03); **C-2** `image_type=raw` (previously in **no** slice); **C-3** `memory.max = guest + reserve` (slices 01, 05); **C-4** the Landlock grant is the vsock socket's **containing directory**, not the three paths CH auto-derives (slice-03); **C-5** the seccomp AC now reads `/proc/<pid>/task/*/status` — **it previously failed against correct behaviour**; **C-6** `RLIMIT_FSIZE = max(rootfs, guest RAM)` encoded from Slice 01 (slices 03, 04); **C-7** a fifth Slice-02 variant, *kernel present but not loadable* — plus Titan's **A-3** (`shared=on` measured x86_64-only) carried onto slice-04. **LOW — ADR-0082's title and D2 heading** said *"unrepresentable"* while the corrected body downgrades to *"private fields + one rendering site + a `dst-lint` clause"*; both, plus brief § 102's heading, now say **"structurally discouraged and lint-enforced"** — the body governs. |
| 2026-08-11 | DESIGN (system/infrastructure — revision after adversarial review, Titan) | Applied the system-designer's three items from the cross-wave adversarial review (fable, `NEEDS_REVISION`: 1 critical, 4 high). **R-C1 (CRITICAL) — the reap becomes a `Reconciler` (Bar 2), per user ruling.** The prior triage ran only the *workflow*-disqualification test and then asserted Bar 1 **by analogy** to `veth_provisioner::provision`; the Bar-1-vs-Bar-2 test (*does `actual` drift while the system is up?*) was never run, and its honest answer is **yes** — a clone leaked by a crash between teardown steps, a scope/run-dir stranded by a failed stop (leaving the VM **unstoppable until the next `serve` restart**), and SD-2's own unbounded clone-leak GC being boot-only so a node whose `serve` never restarts never sweeps it. The review also **settled a claim in the other direction and it is recorded as false**: `setsid(2)` detaches session and process group, **not parentage**, so the VMM *is* a child and `serve` **does** observe its mid-run exit — *"detached, therefore invisible, therefore Bar 2"* must not be used. Three consequences carried: (a) the two regimes are now a **safety** distinction — the boot epoch may reclaim every VM allocation with surviving host state, a steady-state tick may reclaim only terminal/unknown allocations and stranded artifacts, and the supervision discriminator must be an **observed** input, never a View marker; (b) boot ordering resolved — a broker-driven loop has no bootstrap sweep, so the boot-epoch pass is a **synchronous inline convergence** between `Vmm::probe()` and `adopt_on_restart_recovery`, carrying the rmdir-settled obligation, with the reconciler **registered after** the boot passes and **not** gated on `Vmm` composition; (c) the bar change's cost stated — a `State` hydrated from host surfaces, a field-less `View`, ESR specs, DST reachability, and **≥1 new `Action` variant plus executor**, which falsifies the domain wave's *"no new `Action` variant"*. **Founds-the-shared-model question answered explicitly: it does NOT.** It ships a concrete instance and sets the precedent; generalising across #197/#198/#199/#234's four differently-shaped resource classes from one datapoint is speculative generality, and #197 stays the home — the price is a fifth migration site, mitigated by requiring the host-observation hydration to be a named, separable step. **R-H1 (HIGH) — the *"~2.5 s to full guest RAM residency (P13/P14)"* citation is withdrawn**: it is P13's `ondemand`-**restore** uffd backfill applied to the cold-boot path this feature ships, and this design's own P5 datapoint (`VmRSS 276,888 kB` at beacon with 128 MiB touched) refutes it. Cold-boot residency is now stated qualitatively (workload-dependent, trending toward full via the guest page cache, unmeasured), and SD-4's decision is re-grounded on the residency-**independent** charging argument. Handed to Hera: the same figure inflated D-3's urgency framing. **R-H3 (HIGH) — A-3 labelled**: `--memory shared=on` is measured on **x86_64 only** while Slice 04 designs the volume path for both shipping arches; sensitivity — **if `shared=on` misbehaves on Arm metal, Slice 04 is x86_64-only until measured**. The estimation preamble now carries three labelled assumptions (was two, and A-3 was labelled nowhere) and `brief.md` carries them as A-1/A-2/A-3. **Out of lane, deliberately untouched:** R-H2 (the graceful-shutdown *"Proven: P2, both arches"* overclaim), R-H4 (the unlanded C-1…C-7 slice-text corrections), the ADR-0082 title/heading overclaim, and every residual Bar-1 / "boot pass" phrasing inside the domain and application sections — each named in the handoff for its owner. |
| 2026-08-11 | DESIGN (application — review fixes, round 2) | Iteration-2 review verified 7 of the 16 iteration-1 findings landed cleanly (C2, H1, H3, H4, H6, H7, H8 + the `SeccompMode` collapse) and found **the fixes reproduced iteration-1's own diagnosis — "reasoned well about *values*, badly about *composition*" — inside the composition fixes themselves.** Three of the four each stopped **one seam short**, each invisible without reading the cited code. **CRITICAL ×4, all mechanical, none re-opening a decision.** (1) **C1 landed in the trait block but not in the § D6 edge-case table — the one table the ADR calls *"pinned so they cannot be interpreted"* and the one `vmm_equivalence` asserts** — which still named `shutdown` and the deleted `VmShutdownOutcome`, plus two stale copies in the SSOT (brief.md's edge-case prose and the C4 L2 mermaid node). A crafter implementing that table verbatim writes a type the ADR deleted. (2) **C4's "one observer per registry entry" hits a FIFTH seam the four-consumer table does not name**: `ServerHandle` holds a **scalar** `exit_observer_task: JoinHandle<()>` (`lib.rs:1020`) awaited once (`:1135-1136`), and the token is minted per call (`:2290`). A naive loop **detaches** N−1 tasks (dropping a tokio `JoinHandle` does not abort), so they outlive `shutdown()` holding `Arc` clones and obs writes land after teardown; and per-driver tokens leave N−1 tasks parked on `rx.recv()` with **no cancel path** — the exact deadlock the token exists to prevent. Pinned: `Vec<JoinHandle<()>>`, cancel-once-then-await-all, and the loop **clones** the single token. (3) **C6 miscounted the precedent it copied.** The exit observer submits **four** evaluations per exit, not three — `workload_lifecycle` (`:234`), `backend_discovery_bridge` (`:254`), `service_lifecycle` (`:295`) and **`svid_lifecycle` (`:318-320`)** — and the reap submitted only the first. `svid_lifecycle` converges `¬running ∧ held → DropSvid` (`svid_lifecycle.rs:316-317`, `:506-513`) and `exit_observer.rs:318-320` is its **only** on-exit producer, so a reap-authored terminal row would flip `desired` to ¬running with nothing enqueuing it: **`DropSvid` never fires and the node keeps the dead allocation's leaf private key** — ADR-0067 O2's leak-resistance property broken on every `serve` restart that reclaims a VM. All four are now submitted. (4) **C5's pinned signature was mis-cited and unimplementable**: `action_shim::dispatch` is declared at `:671` (`:852` is an *argument* inside `dispatch_with_workflow_intent`'s call at `:842`), and the index is read/written inside **`dispatch_single`** (`:983`), whose signature was not pinned at all — so a crafter had to invent a parameter on the one function the ADR calls the `[G1]` pass/fail bar, for exactly the reason it gives for pinning it. Both signatures now pinned, plus `AllocDriverIndex`. **HIGH ×6:** the relocated guest half had **no pinned shape and no bound** — `Driver::stop` has no `grace` parameter, so `VM_SHUTDOWN_REQUEST_DEADLINE` (2 s) and `VM_STOP_GRACE` (10 s) are now pinned with derivations, **without which this ADR's own claim that an unresponsive guest still lands `Terminated/Stopped{Operator}` had no mechanism**; `LiveVm.beacon: Option<BeaconSession>` names the session and makes the pre-beacon window the only other inhabitant rather than a case to remember; and the enforcement gap the relocation created (`vmm_equivalence` drives the port and structurally cannot reach `VmDriver::stop`) is closed with a named `VmDriver`-level acceptance case. The C3 fix **diagnosed** the "enumerated `ServiceLifecycle` exhaustively, did not do the same for `WorkloadLifecycle`" asymmetry and then **still did not enumerate** — all five `terminal: Some(..)` emitters are now tabled (`:390-393`, `:439-442`, `:515-518` unchanged, each filtering `state == Running`; `:635-638` and `:703-706` guarded). The binding count contradicted itself **three ways in one PR** (ADR Positive said "three lines", ADR Negative said "four, not three", brief said "five") — one convention now applied everywhere. The DISTILL handoff's mutation list was **the only surviving copy of both fixed defects** — it still opened with `reserve_bytes` (the H5 finding) and closed with `VmConfinement::confined`'s `Enforce` literal (a site on the enum the MEDIUM fix deleted); replaced with the SSOT's list. `alloc_drivers` was under-specified on **lock discipline** (read sites immediately `.await` a driver call — the "never hold a lock across `.await`" trap, and `dispatch` already carries both `tokio` and `parking_lot` mutexes so there was no default) and on **lifetime**; both pinned. Six new citations introduced by the round-1 fixes were wrong and are corrected — most importantly **the settle contract's own evidence** (`veth_provisioner.rs:1984-1997` is `alloc_scope_pids`, not `adopt_on_restart_recovery` at `:2099`), which had made the H2 mechanism unverifiable by the next reader. **MEDIUM ×4:** three of four LOW citation corrections had landed **only in the changelog** and not in any normative text (`action_shim:1300`, `workload_lifecycle:1088`/`:1113`, `lib.rs:1401`) — a changelog asserting a fix that landed in one of four files is worse than no entry; `aggregate/mod.rs:909` likewise; the `AllocationSpec` blast radius is **ten**, not eleven; and *"fired unconditionally"* was literally false of `MtlsInterceptWorker` (it is double-gated on `Running` and `mtls_worker.is_some()`) — the true finding is *no driver-type gate*. **Verified closed by this round:** P2 is correctly scoped per-`alloc_id`, so a legitimate `FinalizeFailed` for a different alloc in the same tick does not trip it. **Escalation:** iteration 2 of a maximum 2. The reviewer's recommended disposition — apply as a directed patch rather than a third round — was followed; **the escalation threshold is surfaced to the user below.** |
| 2026-08-11 | DESIGN (application — review fixes) | Adversarial review (Opus) returned **`rejected_pending_revisions`** with **6 critical + 8 high + 3 medium + 2 low**; all fixed, none waived. It independently verified ~20 code citations correct — including the two findings the design rests hardest on (`service_lifecycle.rs:956-996` has no `AllocState` gate; `TransitionReason::NoCapacity` has zero production construction sites) — and confirmed no invented issue numbers and no scope creep from #96/#97/#100. **The dominant theme: the design reasoned well about *values* and badly about *composition*.** **CRITICAL ×3 on composition — `AppState.driver` has FOUR consumers and only one was specified.** (a) `exit_observer::spawn_with_runtime` is called **once** (`lib.rs:2293`) with the single driver; `take_exit_receiver()` yields *the one* receiver and returns early on `None`, `driver_kind` is captured once (`:171`) and stamped on every row, and `ExitEvent` carries no driver discriminator — so under a registry **VM `ExitEvent`s would never reach the ObservationStore and `[D3]`, the feature's north star, would be dead on the production path.** Fixed: one observer task per registry entry. (b) `Action::StopAllocation` and `Action::FinalizeFailed` carry **no spec and no `workload_id`** (`reconcilers/mod.rs:411-416`, `:448-453`), and `AllocStatusRow.kind` is `WorkloadKind` — which the design itself pins as *not* the driver — so **the stop path had no routing key at all**, and `ExecDriver::stop` on a VM alloc returns `NotFound`, which `let _ =` swallows: the GiB-scale unstoppable orphan SD-1 exists to prevent. Fixed: `AppState.alloc_drivers` index written on Start/Restart, plus `action_shim::dispatch`'s signature **pinned** (leaving the one function that IS the `[G1]` bar unpinned while pinning three others was the inconsistency). (c) **`execute_vm_reap` had no wake mechanism** — `spawn_convergence_loop` is purely broker-driven with no bootstrap sweep, and the reap bypasses the exit observer *by design*, so SD-1's *"lets the existing restart/backoff reconciler re-drive them"* had **no mechanism**; the pinned signature made submission structurally impossible. Fixed: the broker is a required parameter and the reap submits one evaluation per terminal row. **CRITICAL ×2 on pinned signatures that could not be implemented as written.** `Vmm::shutdown(&VmControl, grace)` was specified as *"write `SHUTDOWN` on the already-accepted beacon connection"* — but `VmControl` is `{pid, api_socket}`, the beacon listener is `VmDriver`'s (in `overdrive-worker`) and `CloudHypervisorVmm` is in `overdrive-host`, so **the adapter had no handle to the connection**, and `SimVmm` could not honestly return `GuestPoweredOff`, hollowing out the `vmm_equivalence` test that is the design's own enforcement. Fixed by following ownership: re-scoped to `Vmm::terminate` (process half) with the guest request moved to `VmDriver::stop` — **and the window the first draft never named is now pinned**: a stop arriving *before* the guest beacons has no session to write to, so step 1 is skipped and the alloc still lands `Terminated / Stopped{Operator}`. And `VmExitWatch::recv(self)` **destroyed the exit watch on the success path** — a by-value `recv` moves the receiver into the select arm, so when the beacon arm won the receiver dropped, the adapter's `send` failed, and the VMM's exit was never observed; it also would not compile, since the partial move blocks the `Ok` arm from handing the watch to the watcher, which is exactly what the design's own `biased;` rationale requires. Fixed: `recv(&mut self)`. **CRITICAL ×1 — the DD-1 binding was incomplete on the very branch the fix routes rows into.** `workload_lifecycle.rs:679-708` is a **second** `FinalizeFailed` emitter: `restart_counts` accumulate across genuine prior failures (`RestartAllocation` reuses the alloc_id, `:744`), the idempotency guard at `:687` reads `failed.terminal` which a reclamation row carries as `None`, so a workload that had already failed five times and is then reclaimed gets a fabricated `BackoffExhausted` — violating DD-1 and falsifying this design's own claim that reclamation causes no cascade. The first draft enumerated `ServiceLifecycle`'s five emit sites exhaustively and **did not do the same for `WorkloadLifecycle`** — the identical error Hera's own review caught at iteration 2. Fixed with a fourth guard, **and the enforcement replaced**: the proposed predicate-disjointness proptest (**P1**) is structurally incapable of catching a missed emission site, so an emission-level property (**P2** — no `FinalizeFailed` for a reclaimed alloc, over both reconcilers) is now the binding one. **HIGH ×8:** `MtlsInterceptWorker::start_alloc` fires unconditionally and its docstring's predicate (`DriverType::Exec`, *"unconditionally true on the worker's exec lifecycle path"*) is falsified by a second driver — fail-closed, so a VM alloc was either killed or given a silent false confidentiality claim; now gated on `Exec` with #222 as the removal condition. `"unrepresentable"` was **overclaimed** — private fields + one site + a *proposed* lint is not a type-level guarantee, and `DiskAttachment` was shown with no constructor at all, which CLAUDE.md forbids a crafter from inventing; language downgraded, constructors pinned, and the three lint clauses promoted to **Slice 01 deliverables with an AC**. `reserve_bytes` was listed as a mandatory Slice 01 mutation target **and** as a `todo!()` — a vacuously satisfiable gate; its obligations now attach at the DELIVER step that measures it. `VmConfig::rlimit_fsize` was declared pure with universe ∅ while being `max(rootfs size, guest RAM)` — a `stat(2)` wearing a pure signature; `RootfsPlan` now carries `master_bytes` captured at construction (the same Functional-Core split `KernelImage::validate` already uses). `Vmm::probe` scenario 5 required an **fstype check three paragraphs after condemning fstype checks**; dropped, because the reap needs the run dir's *absence after reboot* (directly observable) and never its fstype. §111's confidentiality scenario **misattributed P5** to a guest when P5 tested a host-side process, and dropped P5's own caveat (*"the same path set CH was given, not a byte-copy of CH's internal ruleset"*); restated at the layer measured, caveat carried. The reap/adopt interaction was analysed for ordering but not for the **read race** — `adopt_on_restart_recovery` refuses the boot on any non-`NotFound` read error, which a scope mid-deletion produces; a settle contract now binds `execute_vm_reap`. §108's effect-isolation table omitted six components; added. **MEDIUM:** the `SeccompMode` three-variant compromise was a **rationalisation** — the *renderer* is a mutation site regardless of cardinality, so `seccomp_arg()` satisfies Slice 01's AC **and** makes `Off`/`Log` unrepresentable; withdrawn, and the design no longer abandons its own governing rule anywhere. `aggregate/mod.rs:909`'s `// Future: MicroVmInput` added to the doc-correction list. **LOW:** four citation imprecisions corrected — `lib.rs:1401` (a fn signature) → `:1422-1425` (the actual composition, which Slice 01 already cited correctly, and on which the ADR had built a pass/fail bar repeated three times); `action_shim/mod.rs:1300` (blank) → `:1301`; `workload_lifecycle.rs:1088`/`:1113` (docstring starts) → `:1096-1111`/`:1116-1120`, matching Hera's own table one section earlier. **Lane discipline reviewed and cleared:** the DD-5 universe extension by `last_failure_seen_at` over-declares rather than under-declares (the safe direction under the effect-isolation mandate) and is stated rather than smuggled; `AllocState::Terminated` was already pinned by Hera's own DD-5 row and is restated, not decided. One residual now recorded: an alloc with a **prior genuine failure** keeps a stale `last_failure_seen_at`, costing at most one already-elapsed backoff window. |
| 2026-08-11 | DESIGN (application / component — Morgan) | **Third and last of three DESIGN dispatches**, consuming SD-1…SD-5 and DD-1…DD-6 without amending either. Verdict: **application design was warranted, and this is the wave with the most to build** — a repo-wide search confirms **no Rust code for this feature exists** (one comment and two pre-existing `DriverType` variants), and three code surfaces the prior waves' decisions terminate in are load-bearing for the feature's own pass/fail bar. Recorded in `brief.md` § *Application Architecture* as § 99–114, plus **ADR-0082** (`Vmm` port + `VmConfig`) and **ADR-0083** (`DriverRegistry` + per-driver payload + DD-1 binding). **`Vmm` is four methods** (`kind`/`probe`/`create`/`shutdown`) — explicitly **not** the reference implementation's `configure → set_boot_source → attach_drive → start` state machine (intake I-2's caveat); `CloudHypervisorVmm` in `overdrive-host`, `SimVmm` in `overdrive-sim`, `VmDriver` in `overdrive-worker`, every port a **required** ctor param (no builder). **`VmConfig` makes five substrate lies unrepresentable rather than documented**, under one rule — *for each lie the field a crafter could get wrong does not exist; the correct value is computed from a field that cannot be omitted*: `DiskAttachment` has no `image_type` field and renders `image_type=raw` unconditionally on the value (C-2); `VmRunDir` owns every path inside itself and derives the `access=rw` **directory** grant CH does not auto-derive, making SD-2's exclusivity structural (C-4); `MemoryPlan::derive` is the only constructor, so `guest == cgroup_max` is not representable (C-3/SD-4); `rlimit_fsize()` is `max(rootfs, guest RAM)` encoded from Slice 01 (C-6); `KernelImage::validate` is a **pure** arch-parameterised magic check running before CH sees the file (C-7); and the clone uses the **`FICLONE` ioctl directly**, so there is no `--reflink=auto` path to degrade (C-1). **`reserve_bytes` ships as a RED scaffold — the hardest DELIVER dependency this design creates** (both known floors are RSS-derived while the cgroup also charges page tables RSS cannot see; guessing between them is intake-precedent-#7 in different units). **Three-way race pinned with `biased;`** (Titan named the arms, not the tie-break) and **every non-`Ok` arm cleans up including the deadline arm**. **`DriverRegistry` executes ADR-0022's pre-committed migration** and **IS SD-5's capability gate** — absence of a `Vm` key and the admission rejection become the same fact, so they cannot disagree; option B (a `match` + a bool beside it) rejected on exactly that. `AllocationSpec.command`/`.args` → `driver: DriverPayload` (no serde/rkyv ⇒ **no envelope bump**); `ParseError::MissingExec` deleted for `MissingDriverSection`/`MultipleDriverSections`; `classify_driver_failure`'s documented-but-unused `DriverType` param cashed with zero exec cases moved. **Twelve `TransitionReason::Vm*` Cause variants named** — Hera's re-assignment delivered, disposition deliberately excluded from K3. **DD-1 bound at three lines across two reconcilers**, with `is_intentionally_stopped` and `is_restartable` needing **no** change (`PlatformReclaimed` fails the former for free and therefore satisfies the latter for free); totality is a **proptest**, `EndingClass` rejected as disproportionate; `CrashFacts::advance` **unchanged**. Shutdown reuses the guest's open vsock connection — **CH's `vm.power-button` rejected** because a ~200-line PID 1 has no `acpid` and aarch64 uses PSCI. Reuse: **25 rows**, 6 CREATE-NEW all pre-ratified, **zero new third-party dependencies**; the `Driver` trait is **unchanged** — intake I-2's licence to change it deliberately not exercised. **No contradictions with Titan or Hera**; five sharpenings (notably: the reap must run *outside* `adopt_on_restart_recovery`'s `mtls_worker.is_some()` gate, and is deliberately **not** `Vmm`-gated so a node that uninstalled CH still reclaims survivors) and two extensions (DD-5's universe gains `last_failure_seen_at`; the reap becomes plan-value). **Two open DESIGN inputs closed rather than returned**: Slice 03's uid/gid (P5) and Slice 04's `[[vm.volume]]`-in-the-aggregate (rides Slice 01's V2). **Slice 04's lift trigger re-run at the DESIGN handoff as required: it does not fire** — this design reduces the estimate and eliminates the conditional second rkyv bump. **Five new deferrals surfaced for user approval; no GitHub issues created** — M-1 (the reserve measurement), M-2 (`dst_lint`'s `BANNED_APIS` covers neither `Command` nor `UnixListener`, so the lint would **not** have blocked a portless driver — stated so the port's justification rests on the rule that genuinely forces it), M-3 (ADR-0022 partial supersession), M-4 (**ADR-0081 left reserved** for Hera's H-1), M-5 (`overdrive-init`'s two musl targets + publication). |
| 2026-08-11 | DESIGN (domain / bounded-context — Hera) | **Second of three DESIGN dispatches.** Verdict: **no new bounded context and no new aggregate** — the reuse table demonstrates it rather than asserting it (14 existing components assessed; **CREATE NEW is empty of structure** — the entire domain delta is new variants on two `#[non_exhaustive]` vocabularies plus one clause on one existing predicate). A `VmInstance` aggregate, a VM bounded context, a new `AllocState` and ES/CQRS were each considered and rejected with a stated reason; the only candidate invariant (guest RAM vs cgroup limit, SD-4) is a **start-time derivation**, so persisting it would have manufactured the invariant that then justified the aggregate. Recorded in `docs/product/architecture/brief.md` § *Domain Model* as **DD-1** (an ending classifies into **three** classes — Intentional Stop / Workload Failure / **Platform Reclamation** — and restart eligibility, budget consumption **and** job finalisation are all functions of the class), **DD-2** (the reclamation's durable surface is ADR-0078's `LastTerminated` + `restart_count` **unchanged**, and the exemption applies to the **budget** only — two different quantities that one English word covers), **DD-3** (the reason vocabulary has **two axes** — Cause and Disposition — so US-VM-2/K3 scope to Cause only; and `TransitionReason::OutOfMemory`'s missing emit site is a **declared hole**, modelled with its discharge condition and **not resolved**, per the dispatch), **DD-4** (four pinned terms: Workload Kind vs Workload Driver, Restart Budget vs Restart Count, Platform Reclamation, and what `CleanExit` means for a VM), **DD-5** (`Job`'s boundary unchanged, checked against Vernon's four rules; bounded-change contracts pinned per command so the **budget exemption ships as a complement-equality assertion, not a comment**), and **DD-6** (context map — one Core context, **ACL** to the Hypervisor Substrate and **Published Language** to the Guest Runtime, each label evidence-backed; ES/CQRS assessed **NO** across the board with the depth-1 trade-off stated). **One failure SD-1 does not name, found here and reachable only once SD-1 lands:** a merely-"distinct reason" reap row still satisfies `is_natural_exit` (`workload_lifecycle.rs:1124-1131`), and the Job-kind finalise branch (`:622-624`) runs **before** the restart branch (`:673`), so a reaped **Job**-kind VM is finalised `TerminalCondition::Failed { exit_code: Some(0) }` — a **fabricated exit code on a workload that never exited** — and is never restarted. Fixing the budget alone converts a cascade into a silent lie. Also established: reusing `StoppedBy::SystemGc` for the reap would leave **every VM dead after a `serve` restart** (the exact inverse of SD-1's intent, reached by taking the nearest existing word), and `CrashFacts::advance` must **not** be "fixed" to exempt reclamation — that would erase the occurrence ADR-0078 exists to preserve, though its *"unreachable in Phase 1"* clause narrows and must be amended in the same commit. SD-1's Bar-1 triage independently re-derived against `workflows.md` / `reconcilers.md` and **agrees**. **No contradictions with SD-1…SD-5 or the spike**; three sharpenings and one extension recorded. **Four items surfaced for user approval; no GitHub issues created** — H-1 (DD-1 is ADR-worthy and platform-wide, not VM-specific; next free number is **ADR-0081**), H-3 (`ADR-0031:539` names a `[microvm]` table intake I-5 deleted), H-4 (`NoCapacity` is the second declared hole — fold into D-2). The § *System Architecture* section was not touched. |
| 2026-08-11 | DESIGN (domain — review fixes, round 2) | Iteration-2 review verified 12 of 13 iteration-1 findings closed with corrected line numbers, and found **the round-1 fix to HIGH-1 was itself incomplete** — plus one scope regression it introduced. **HIGH — reuse row 16 certified `ServiceLifecycle` benign from ONE of its five action-emitting sites.** The liveness branch (`service_lifecycle.rs:769`) *is* state-gated and is genuinely unreachable for a reclaimed alloc; but the enclosing loop at `:500` filters **no** state, and `startup_probe_failed_action` (`:968-991`, emitted `:651-658`) gates only on `started_at.is_some()` ∧ attempts ∧ deadline ∧ no-Pass with **no `AllocState` gate at all** — so a Service alloc reclaimed after Running but before Stable is handed a fabricated `ServiceFailed { StartupProbeFailed }` for probes that never failed. **That is DD-1 trap 3's shape recurring on the Service path, on the component the fix had just declared clear** — certifying a component from one branch being the same error as certifying an ending from one predicate. Row 16 re-verdicted **EXTEND** (budget half still clear: `restart_counts` has exactly one writer workspace-wide, `workload_lifecycle.rs:788`), and **DD-1's rule restated in its general form** — *no reconciler may author a terminal claim on a Platform-Reclamation row* — with the binding-sites table now spanning **two** reconcilers instead of listing `WorkloadLifecycle` predicates. This is the round's most valuable change: the general form is enforceable against reconcilers that do not exist yet. **MEDIUM — DD-5 pinned `state → Terminated` while DD-1's boundary note declared the state a free surface question**, an internal contradiction *and* lane creep. Resolved by tightening DD-1 rather than loosening DD-5: **`AllocState::Failed` is excluded on domain grounds** (it asserts a run ended that did not end — the misclassification this feature refuses, written into the reclamation itself) **and on a mechanical ground the review surfaced** — `service_lifecycle.rs:611` gates its EarlyExit branch on `state == Failed` and fabricates `ServiceFailed { EarlyExit { exit_code } }` at `:631-636`, manufacturing an exit code for a workload that never exited. `Terminated` is now *indicated* with the selection test stated (*does any reconciler failure branch key on this state?*), so the choice is constrained by the domain rather than left free and then silently assumed. **LOW ×2:** row 16 cited `Action::RestartAllocation` at `:729-731` (a docstring) — corrected to `:798`; and LOW-4's `advance` pointer fix had been applied in `brief.md` but not propagated to reuse row 6 — now `:1144-1159` in both. A fourth acceptance scenario added (the Service-path analogue of the Job-kind fabrication), with its reachability qualified: `[vm]` + `[service]` is rejected at deploy until #257, so it is an exec-Service case today — covered anyway, because the reclamation class is not VM-specific. Regression sweep otherwise clean: 16 rows and the "sixteen" count agree, "CREATE NEW: nothing structural" still holds (both new rows are reuse), and every new assertion in the round-1 prose resolved against code. |
| 2026-08-11 | DESIGN (domain — review fixes) | Adversarial review (Opus) returned **`rejected_pending_revisions`** with 0 blockers, 3 HIGH, 5 MEDIUM and 5 LOW; **all fixed, none waived.** It independently verified the headline DD-1 trap-3 discovery in all three legs — the finalise branch `return`s at `:632`/`:639` before the restart branch at `:673`, both iterating the same `active_allocs_vec`; the `:626-633` idempotency guard only short-circuits on an existing `Completed \| Failed` terminal, which a reclamation row would not carry, so **the fabrication path is live, not guarded** — and verified DD-2's same-LWW-key mechanism, DD-4's `WorkloadKind`/`DriverType` pins, and that DD-5's 15 declared slots exactly exhaust `AllocStatusRowV3`'s 15 fields. **HIGH ×3, all on the hard gate or its evidence:** (1) the Reuse table omitted **`ServiceLifecycle`**, a *second* reconciler over the same `RESTART_BACKOFF_CEILING` that emits its own `RestartAllocation` (`service_lifecycle.rs:729-731`) and `FinalizeFailed` (`:779-787`) — added as row 16, `REUSE UNCHANGED`, with the two facts that make it benign recorded rather than assumed (its docstring at `:196-198` says it *consumes* `restart_counts` and never increments, so DD-1's exemption is automatically consistent; and `:769` gates on `state == Running`, which a reclaimed alloc is not); (2) the table omitted **`ExitEvent.intentional_stop`** (`traits/driver.rs:278-303`), which is *in-tree* described as the load-bearing two-class ending discriminator — i.e. the exact place a crafter would try to put a third class — added as row 15 with the discharge DD-2 now also carries: after a `serve` restart `ExecDriver.live` is reconstructed empty, so **no `ExitEvent` is produced for a surviving VMM at all** and the reap authors its terminal row directly; (3) `TransitionSource::Driver(DriverType)` was cited at `api.rs:622` (an `AllocStateWire` docstring) — corrected to `:710`/`:714`, which matters because it is the evidence for DD-4's *"not persisted on any row"* pin. **MEDIUM ×5:** DD-1's constraint *"derivable from the terminal row alone"* was justified by *"(the reconciler holds no other input at that seam)"* — **false**, `WorkloadLifecycleState.job` is read in the same match arm at `:734`/`:742`/`:750`; the parenthetical is deleted and the constraint re-justified on **generality** (a driver-derived class is one only VMs can be in, and drain/eviction/migration are reclamation on non-VMs). **`NoCapacity` was mislabelled "the second declared hole" and the stronger finding was available:** `transition_reason.rs:55` marks it emitted **`yes`** against no production construction site — an *undeclared* hole plus a **false doc claim**, the same violation DD-2 already obliges to be fixed for `advance`'s clause; reclassified, split into an in-scope fix (H-4a) and D-2's mechanism half (H-4b), and DD-3's closing rule sharpened to distinguish *a word the system cannot say* (a hole) from *documentation claiming it can* (a lie). DD-5's "Command" column named `ReclaimAllocation` beside two real `Action` variants, inviting a crafter to mint `Action::ReclaimAllocation` against the wave's own "no new type" claim — each row now states its actual **or deliberately absent** code surface, with the CREATE NEW list extended to say "no new `Action` variant" explicitly. DD-4's microvm-drift sweep named only `ADR-0031:539` and missed **`aggregate/mod.rs:166`** — a `MicroVm(MicroVm)` forward pointer *inside `WorkloadDriver`*, the enum this feature adds `Vm` to; added, and classified in-scope (the commit that adds the variant is the commit that makes it false) rather than deferred. Titan's handoff item 3 (variant naming) was declared "settled … needs no user input", conflating *no user ruling* with *delivered* — this wave now states plainly that it delivers the **Disposition** name and C-7's **meaning**, and **re-assigns the Slice 02/03/04 Cause-variant naming to the solution architect**, bounded by DD-3's two-axis rule. **LOW ×5:** five citation imprecisions corrected (`is_intentionally_stopped`'s dropped `state == Terminated` conjunct — which is what makes its set asymmetric to `is_restartable`'s; `AllocState` cited at its `is_terminal` body rather than the enum; `TerminalCondition`'s range truncating before `Completed`/`Failed`, the two variants trap 3 turns on; `advance` at `:1144` not `:1140`; and DD-5's non-terminal→terminal forward-carry cited to the *terminal→terminal* edge case). All five conclusions were independently verified correct; only the pointers moved. |
| 2026-08-10 | DESIGN (system/infrastructure — review fixes) | Adversarial review returned **`rejected_pending_revisions`** with 1 blocker + 5 high; all fixed, none waived. **BLOCKER — SD-5's sole precedent was factually inverted.** The draft claimed `EbpfDataplane::probe` failure *"emits `health.startup.refused` at `warn!` and boot continues"* and built a **capability-refusal** disposition on it. `lib.rs:1681-1693` emits at `warn!` **and then `return Err(ControlPlaneError::DataplaneBoot(..))`**, under a comment saying *"refuse to boot"* verbatim — the logging level was misread as a disposition. **There is no in-tree precedent for a probe that fails and lets the node start** (all six refuse, or are never called), so the claim had followed an inference rather than the code. **The recommendation changed**: SD-5 is now a **composition-gated hard refusal** — a substrate *lie* refuses the node (uniform with all six precedents), a capability *absence* does not, and the composition gate keys off the hypervisor binary's presence rather than a new operator knob. **HIGH ×5:** (1) the *"~100 ms CH failure-exit"* figure carrying SD-3's entire case for option B over A **appears nowhere in the spike** — the probes recorded CH's exit *status*, never its *latency*; restated as a labelled assumption with its sensitivity named (*if wrong, option C should be re-opened*) and added to DELIVER's measurement list, alongside the estimation block's claim that *"nothing is a gut feel"*, now withdrawn; (2) the `ExecDriver::start ~1 ms` baseline was likewise unsourced **and inverted an ADR-0054 property** — `driver.rs:172` states *"no direct `tokio::fs::*` calls from `driver.rs`"*, so the writes go through the `CgroupFs` port; corrected, and the absolute value marked unmeasured and unused; (3) **SD-1 invoked `reconcilers.md` Bar 1 and then specified an imperative four-step reap** — the half-provisioned-resource bug that rule exists to prevent, reproduced in the headline decision while citing the rule as cover; respecified as observe → diff → converge with the observed surfaces named, an authority rule for when the cgroup scope and the run directory disagree, per-step idempotence, and every partial-crash state's convergence stated; (4) SD-2 **named an unbounded rootfs-clone leak, quantified it, and assigned it to nobody** — Slice 03's terminal-state GC does not cover reboot-orphans, since no allocation remains to key off; **folded into SD-1's boot pass** (which already walks the allocation set), making the clone filename's allocation id load-bearing; (5) the Reuse table **omitted the composition-root driver dispatch** (`lib.rs:1422-1425`) that SD-5 acts through — added as row 13, EXTEND. **Factual corrections:** the probe inventory said *"five traits"* and named four, one of which (`EbpfDataplane`) is an inherent method on a struct — the five Earned-Trust **traits** are `ViewStore`/`JournalStore`/`CgroupFs`/`MtlsEnforcement`/`MtlsResolve`; *"never constructed anywhere in the tree"* → **no production emit site** for both `OutOfMemory` and `NoCapacity` (both exist in tests; the live `PlacementError::NoCapacity` is a different type); **C-1 re-marked as an inference** (P4 measured `auto` succeeding on capable XFS and `always` failing cross-device; the silent-full-copy degradation is stated counterfactually in `findings.md` and goes on DELIVER's measurement list) and **C-3 re-marked as arithmetic**, with its timing corrected from "Slice 01" to "whichever slice first derives guest RAM from `memory_bytes`" — which is still a Slice 01 decision either way; SD-4's *"the spike does not isolate VMM overhead"* corrected; **P5's three corrections now cited by content**, because `findings.md` and `wave-decisions.md` number them in **different orders**; D-3's tension with `[D3]` named so the user rules on it knowing a cgroup OOM will ship misclassified as `signal: 9`; the `Vmm::probe` signature de-pinned to a constraint (lane: signatures are solution-architect's); `eval_broker.rs` re-homed to `overdrive-core` with `cancelable`'s unbounded growth noted; `tick.deadline` restated as read by the DST harness but by no production reconciler or runtime code; `spawn_exit_watcher`'s line range extended to cover the `classify_exit` the claim rests on. |
| 2026-08-10 | DESIGN (system/infrastructure — review fixes, round 2) | Iteration-2 review closed the blocker and all five HIGHs, and found one **regression introduced by the round-1 fixes** plus six precision defects; all corrected. **REGRESSION — SD-4's replacement overhead figure was arithmetically wrong.** Round 1 replaced *"the spike does not isolate VMM overhead"* with *"`Rss − Private_Dirty` = **~4.4 MB above guest RAM** … the vCPU-stack + ring + API-server + text + page-table bundle, isolated."* Both halves are false: `Private_Dirty` (2,098,144 kB) is already **992 kB above** the 2 GiB guest, so it is the wrong subtrahend; and the 4,540 kB remainder matches `Shared_Clean` (4,536 kB) — it is **the binary's text alone**, which `findings.md` states directly, while the vCPU stacks and rings sit *inside* the private-dirty term. Replaced with two honest floors — **~5.4 MiB** steady-state `VmRSS` above a 2 GiB guest, and **~11.9 MiB** (`VmRSS 12,136 kB`) at zero guest residency — plus the structural point neither round caught: **host page tables are charged to the cgroup via `memory.stat pagetables` and are invisible to RSS**, so DELIVER must measure the reserve via `memory.current` / `memory.stat`, not RSS. SD-4's decision is unaffected; the risk was anchoring a crafter to a number 2.7× off the spike's own better datapoint. **SD-1's authority rule corrected** — it assigned *"was it a VM"* to the tmpfs run directory, contradicting SD-2's own epoch framing (the directory is absent for **every** VM after a host reboot) and leaving *"directory gone, scope populated"* undecidable; keying it on the **non-terminal allocation row** instead confines the reap to VM allocations and leaves `ExecDriver`'s survive-a-restart behaviour for process workloads untouched — which the ambiguous version could have silently changed. **SD-5's precedent census corrected and strengthened**: *"6 of 6"* omitted `ProbeRunner::probe` (`probe_runner_boot.rs:63`) and `DnsResponder::probe` (`lib.rs:2253`) and glossed `JournalStore`'s never-called status; more importantly it under-cited its own best evidence — `MtlsEnforcement::probe` (`lib.rs:1988`) and `MtlsResolve::probe` (`:2021`) sit inside `if compose_mtls` (`:1935`), so **composition-gated hard refusal already ships** and option C *is* the existing pattern rather than an analogy to it. Reuse row 12 narrowed to the prober *traits* so `ProbeRunner`'s boot gate is not dismissed alongside runtime health checks. **The composition gate's inverse hazard named**: installing the CH binary can flip a node from booting to refusing to boot if its staging filesystem cannot reflink — correct behaviour, landing at the next `serve` boot, but an unstated version reads as an unexplained failure after an unrelated package update. Also: the residual *"~1 ms for a process"* removed (it contradicted the same section's statement that the exec figure is unused in any decision); C-7 now acknowledges that Slice 02's unclassified-verbatim arm *does* catch the P1 failure — the defect is that it faithfully reports CH's **misleading** `UefiTooBig` text; the last ambiguous *"P5 correction 1"* cited by content; the clone-filename requirement restated as **chosen** over persisting the clone path on the allocation record, not forced. |
| 2026-08-10 | DESIGN (system/infrastructure — Titan) | **First of three DESIGN dispatches; scope narrowed to Slices 01–05 per the user's 2026-08-10 ruling.** Verdict: system-level design **was** warranted, for exactly five node-level properties — each with a wrong default that ships if undecided, three of them *silent*. Recorded in `docs/product/architecture/brief.md` § System Architecture as **SD-1** (the hypervisor sits outside `serve`'s failure domain; **boot-time reap, never adopt**, because P2 shows one guest-initiated vsock connection carries both the beacon and the exit status, so an adopted VM's ending can never be honestly classified — plus a reap-before-`adopt_on_restart_recovery` ordering constraint and a restart-budget exemption that prevents a node-wide terminal cascade after six `serve` restarts), **SD-2** (per-allocation host state spans two filesystems with *different invalidation semantics* — tmpfs run dir as the durable `alloc ↔ VM` join and the Landlock directory grant; reflink clones on the master's filesystem, needing explicit GC), **SD-3** (a blocking `start()` on a **fully serial, timeout-free** dispatch loop with `TickContext.deadline` never read — bounded inside the driver by a beacon ‖ VMM-exit ‖ deadline race, with the residual `pending × D` stall stated, not hidden), **SD-4** (`memory.max` **cannot** equal guest RAM — the cgroup charges the VMM's entire RSS, so the current shape is cgroup-OOM-by-construction reported as `signal: 9`; reserve is a measured policy function, deliberately not a guessed constant), and **SD-5** (Earned Trust — **seven substrate lies**, a `Vmm::probe()` boot gate, plus per-launch re-enforcement of rows 1–2 so the probe cannot go stale). Mandatory **C4 L1 + L2** diagrams added. **Reuse Analysis (hard gate): 13 existing components assessed** — 6 EXTEND, 1 REUSE VERBATIM, 2 REUSE UNCHANGED, 3 NO CHANGE (two named as gaps), 1 NO REUSE; CREATE NEW limited to the pre-ratified `Vmm`/`CloudHypervisorVmm`/`SimVmm`/`VmDriver` plus `Vmm::probe()` and the reap arm. **Seven spike-versus-slice contradictions found (C-1…C-7)**, the sharpest being **C-5** — Slice 01's seccomp AC reads `/proc/<vmm-pid>/status`, which reports `Seccomp: 0` on a *correctly* confined CH, so **the AC fails against correct behaviour** — and **C-1** (`--reflink=auto` silently degrades ~260× with no error), **C-2** (`image_type=raw` mandatory from v53, absent from every slice), **C-3** (`memory.max` == guest RAM from Slice 01), **C-4** (US-VM-7's ruleset names the three paths CH auto-derives and omits the vsock directory, the only one that needs an explicit rule), **C-6** (`RLIMIT_FSIZE` must be `max(rootfs, guest RAM)` or Slice 04 kills every volume-carrying VM with `SIGXFSZ`), **C-7** (Slice 02 lacks a *kernel-present-but-unloadable* variant, the exact P1 failure with the misleading `UefiTooBig` error). **Six deferrals surfaced for user approval; no GitHub issues created.** Two previously-open DESIGN inputs closed by spike evidence: the hypervisor's uid/gid (unprivileged uid + `kvm` group, no appliance-image change) and the `--landlock` version-floor basis. Explicitly **no** placement, sharding, replication, caching, queueing, CDN or consistency-model design — a single node booting one VM per allocation has none of those problems. |
| 2026-08-01 | DISCUSS | Authored from `intake.md` + three research inputs. Extended J-OPS-003 (no new job). Answered the six intake questions as `[D1]`–`[D6]`. Corrected the intake on the Running gate's location (`[G3]`) and surfaced the unrecorded `Job` rkyv envelope cost (`[G4]`). Scope assessed **OVERSIZED**; split applied (Service-kind out; 5 slices). Four blockers surfaced for user approval. |
| 2026-08-02 | DISCUSS (amendment) | **B1–B4 ruled on.** B1 → #257, B4 → #259, B3 → #258 **and rescoped**, B2 **not approved** (#42 untouched; the dependency is dropped with no replacement pointer). Added **`[D7]`** — the locked isolation claim (*KVM + default-on seccomp + Landlock + cgroup/netns confinement; no jailer-equivalent chroot, no PID namespace*) and the six folded hardening items. Placement: items **5–6** → US-VM-1 ACs (inherent to `VmDriver::start`); items **1–3** → new story **US-VM-7** in Slice 03; item **4** (mount-ns) → **#258**, on shape grounds, pending the user amending #258. Added **K7**; added Slice 00 **P5** (confinement composes with a real boot) and extended **P2** (vsock from inside a netns). Rewrote constraint 6, extended constraint 7 (Landlock gives the first genuine CH version-floor reason). Grounded a new finding: `provision_and_inject_netns` gates on mTLS composition, **not driver type**, so a production VM alloc is handed a netns whether or not the driver enters it. **Scope re-assessed: still right-sized — 5 slices, 7 stories**; Slice 00 → 2–3 d, Slice 03 → 4–6 d, Slice 01 unchanged. |
| 2026-08-02 | DISCUSS (review fixes) | Peer review returned `rejected_pending_revisions` with 1 DoR failure + 6 high issues; all fixed, none waived. **US-VM-1's criteria split** — a 5th UAT scenario added (hypervisor containment), UAT-derived ACs given `*(Scenario N)*` back-references, and the four non-UAT-derived engineering items (`lib.rs:1422`, `MicroVm` deletion, rkyv bump, `SimVmm`) moved to a separate **Engineering Constraints** block; recorded as DoR item-5 note. **Three vacuous-pass traps closed:** US-VM-1 item 5's netns assertion now *requires* an mTLS-composed `serve` (the GH #248 / ADR-0074 trap, reproduced deliberately); item 6 now requires the driver to construct the seccomp argument so the mutation target has a real site; US-VM-7 item 1's Landlock denial now names its executor (Slice 00 P5 for denial evidence; the production half is Example 2's spec-derived ruleset; runtime proof is **#258's** EDD item). **US-VM-7's fail-closed producer named** — injected at the `Vmm` port per constraint 1, since the whole test envelope is one Lima kernel. **`<vmm-pid>` resolution named** (allocation `cgroup.procs`), making US-VM-1's cgroup placement a verification prerequisite for US-VM-7. **`[D7]` staged** — until US-VM-7 lands, only *KVM + default-on seccomp + cgroup/netns* may be asserted. US-VM-7's pitch de-overclaimed (confinement is never rendered to Ana; only its absence is) and its "Before" made counterfactual; K7 scoped to mTLS-composed boots with an `n/a` baseline; rlimit ceiling re-anchored to `/proc/self/limits`; Slice 01's cost note argued rather than asserted. **`docs/product/jobs.yaml` corrected** — its J-OPS-003 entry still described the posture as *"the same cgroup treatment ExecDriver gives workloads"* and the security work as *"currently untracked — surfaced as a DISCUSS blocker"* (a forward pointer with no issue number, in a committed SSOT); now cites `[D7]` and #258, and the service-gap reference cites #257. Same corrections applied to `docs/product/journeys/run-a-vm-workload.yaml`. |
| 2026-08-02 | DISCUSS (amendment 2 — **`I-6` recovered**) | **A dropped user input was recovered.** The user's opening message named two reference decisions — *"speak with CH over unix socket and use virtiofsd for storage"*; the first became `I-2`, **the second was dropped by the intake author and never recorded**, after which research recommended `ext4`+`virtio-blk`, DISCUSS scoped that, and the reversal was never surfaced. Recorded as intake **`I-6`** and as **`[D8]`**: **storage splits by ROLE — `virtio-blk` for the rootfs, `virtiofs` for volumes.** Not a compromise — the reference shipped exactly this split (`architecture.md:196`: block for `code.ext4`/`deps.ext4`, virtiofs only for `attach_drive("output", …)`); the apparent research-vs-reference conflict was an artifact of DISCUSS **scoping the rootfs and never scoping volumes at all**. **Two misattributions purged:** CVE-2026-24834 concerns **`virtio-pmem` + DAX**, not virtiofs, and may no longer be cited against virtiofs (constraint 10); and *"the reference used virtiofsd — the option the most experienced team retreated from"* fused two unrelated facts (Kata retreated from `virtio-pmem`; the reference never put its rootfs on virtiofs). `[D5]` rescoped to the **rootfs only** and its *"not virtiofs"* framing removed — it read as a rejection never decided. **New Slice 04 `vm-writes-output-the-operator-can-read` (4–6 d)**, two stories: **US-VM-8** (the `[[vm.volume]]` capability) and **US-VM-9** (the daemon's lifecycle honesty); the former Slice 04 (resources) renumbered to **05**. Volumes placed above resources on outcome impact (5.0 vs 4.0), not effort; the two are mutually independent. Sub-decisions: **`[D8b]` `--memory shared=on` CONDITIONAL** on volume presence (no cost without benefit; one derived field on a `VmConfig` *value*, not two config shapes; regression-safety for slices 01–03) with **Slice 00 P6 measuring the cost**; **`[D8c]` `--cache=never`** (one guest per share, and no CH DAX to cache against); **`[D8d]` `--sandbox=namespace`, fail-closed, NEVER silently downgraded** — correcting the reference's unrecorded `namespace`→`chroot` drift, precedent warning #6's shape; **`[D8e]`** daemon in the allocation's cgroup scope (so `cgroup_kill` reaps it) and netns, with the volume `source` **excluded** from the hypervisor's Landlock ruleset so volumes do not widen `[D7]`. **`[D3]` generalised into system constraint 9** — *a supervised sidecar's death is classified by the WORKLOAD's outcome, never by the sidecar's exit status* — because the reference got it wrong in **both** directions (a clean `virtiofsd` exit read as `VmmError::Crash` and force-killing the VM; a mid-run death not reaching `ExitKind` at all). **#258 reconciled after its two 2026-08-02 amendments:** mount-ns is **out of this feature** (`[B5]` approved) and **virtiofsd hardening is UNCONDITIONAL**, with an explicit lifecycle-vs-posture boundary table in `[D7]`. **`[G4]` rkyv ruling pinned** — `JobEnvelope` **V1→V2**, full six-step single-commit procedure; the hedges in `[G4]`, Handoff item 3 and slice-01's *"(if required)"* removed. Added **K8** (output fidelity) and **K9** (sidecar-death honesty, both directions); 6 new risks; DoR re-run **9/9 across nine stories**. **Scope re-assessed: right-sized, at the upper edge — 6 slices, 9 stories, 5 modules (unchanged), walking skeleton untouched**; a 7th slice or 10th story is the pre-committed trigger to split into two features. |
| 2026-08-02 | DISCUSS (amendment 2 — review fixes) | Peer review of the `I-6`/`[D8]` amendment returned `rejected_pending_revisions` with **1 critical + 5 high**; all fixed, none waived. **CRITICAL — the amendment scoped the HOST half of volumes and never scoped the GUEST half**, which is *structurally the same omission it was correcting* (`I-6` fell out because DISCUSS scoped the rootfs and never scoped volumes), reproduced one level down. `[D8]`'s first draft wrote *"and mounts the share in the guest at `target`"* — passive, **no subject** — silently expanding `[D4]`'s locked four-duty agent scope. Closed by: a **recorded `[D4]` amendment** adding duty **(e) mount each declared volume before exec, and refuse to exec if a required mount fails**; a new **`[D8g]`** scoping the guest side; a fourth domain example and a sixth UAT scenario for the **composite-lie case** (a silently-unmounted share ⇒ the command writes into the discarded rootfs copy, exits 0, and `workload describe` reports `Completed{0}` over an **empty** host directory — every signal individually truthful, the composite false); a `guest-mount-failed` `TransitionReason`; and an extended BYO-artifact contract (the operator's kernel must supply virtiofs). **HIGH ×5:** (1) **`read_only`'s enforcement point was unspecified** while carrying a security framing — pinned **host-side** in `[D8g]`, with the guest-side `-o ro` demoted to an explicitly non-boundary ergonomic guard and a written fallback (strike the framing rather than ship the guest-side version, per `[D7]`'s precedent); (2) **US-VM-9's clean-exit arm was vacuous** — it passes with zero code written, since a do-nothing implementation also yields `Terminated / Completed` and cargo-mutants cannot *insert* a guard — closed by naming the mutation site (the before-vs-during-teardown discriminator), stating that ACs 1–3 are **one** classification not three independent checks, and requiring the Tier-3 case to assert the daemon's exit **was observed while contributing nothing**; K9's paired inverse fixed the same way; (3) **`[D8f]`'s 4–6 d budget omitted five rows** while claiming to be "budgeted honestly" — re-budgeted to **6–9 d** (parse surface, `VmConfig` payload, guest-side mount, host-side `read_only`, a possible second rkyv bump), making Slice 04 the largest slice after the skeleton and the lift question live; (4) **DoR item 5 FAIL on US-VM-8** — two ACs traced to no scenario, the same *technical-AC* defect fixed in US-VM-1 at iteration 1 — closed by adding scenarios 6–7 and an **Engineering Constraints** block with the surface criterion restated as an observable; (5) **DoR item 9 FAIL on US-VM-5** — a bare `PASS` naming no KPI, and none of K1–K9 measured declared-resource fidelity — closed by minting **K10**. Also fixed (medium/low): the split trigger given a **number (>9 d), an owner (DESIGN) and a checkpoint (the DESIGN handoff)** plus a third, wave-unadjustable trigger (>35 d total, or any slice >50% over its band); the **Value-5-vs-lift-authorisation contradiction reconciled** (if Slice 04 lifts, the honest deliverable is a VM class that computes but cannot deliver, and US-VM-8/9 become a hard prerequisite before operator-facing material calls `[vm]`+`[job]` production-ready); `[D8b]`'s load-bearing reason stated as **regression-safety** with P6's measurement given a **two-directional** consequence; K7 scoped to volume-carrying allocations; Slice 00 P6 extended to probe host-side read-only export, the guest mount + its failure shape, and **volume I/O cost under `--cache=never`** (the measurement discipline applied to the rootfs role but skipped for the volume role); P6's gating corrected to include US-VM-5's both-shapes case; Reading checklist `I-1..I-5` → `I-1..I-6`; outcome `(e)` → `(f)`; US-VM-1's rkyv constraint aligned to slice-01's user-ruled wording; the slice-composition gate qualified to *story-bearing* slices. **Totals after fixes: 6 slices, 9 stories, ~22–32 d, DoR 9/9 with seven recorded notes.** |
| 2026-08-02 | DISCUSS (review iteration 2) | Re-review returned **approved** — all 7 blocking and all 7 medium/low iteration-1 issues verified closed, DoR 9/9, JTBD PASS, slice composition PASS, zero anti-patterns. Its 7 new non-blocking findings were then fixed rather than carried: US-VM-1 Scenario 5's second `Then` re-pointed from stop-reach (US-VM-4's scope, Slice 03) to syscall filtering, which also gives AC item 6 a real assertion to trace to; the seccomp mutation obligation re-anchored to an **argv-level** assertion because **CH's `log` mode still installs a filter**, so a `/proc`-only check kills `false` but not `log`; the rlimit comparison anchor changed from `/proc/self/limits` to the `overdrive serve` process (under a Tier-3 harness `self` is the test process); US-VM-7's fail-closed reason stated as a **fifth** `TransitionReason` variant in the Slice 02 shape, reconciled against US-VM-2's "no two share a variant" and K3's "≥ 4 distinct", and added to journey step 3; the `[D7]` staging rule propagated to `jobs.yaml`; the risks table's `lib.rs:1422` reference re-pointed from "an AC" to the Engineering Constraints block; slice-01's Behavior bullet aligned to the corrected AC wording. |

---

## Wave: DESIGN — adversarial review (Atlas, fable, 2026-08-11)

Reviewer declined to self-apply metadata ("I do not modify reviewed artifacts"); block
appended by the orchestrator verbatim. **Verdict: NEEDS_REVISION** — 1 critical, 4 high.
Spike findings were included as a required source at the user's instruction, and that is
what surfaced both citation defects.

```yaml
review_id: "arch_rev_2026-08-11_design-wave-adversarial"
reviewer: "solution-architect-reviewer (Atlas), model=fable"
artifact: "feature-delta.md §§ Titan/Hera/Morgan, brief.md §§ SysArch/Domain/AppArch, ADR-0082, ADR-0083, slices 01-05"
iteration: 1
approval_status: "rejected_pending_revisions"
critical_issues_count: 1
high_issues_count: 4

critical:
  - id: R-C1
    issue: >-
      BLOCKING NON-COMPLIANCE with user ruling — the VM reaper is specified as Bar 1
      (converge-on-boot) in every artifact; the user has ruled Bar 2 (a Reconciler).
      The triage at brief.md:1034 runs ONLY the workflow-disqualification test and then
      asserts Bar 1 by analogy to veth_provisioner. The Bar-1-vs-Bar-2 test ("does actual
      drift while the system is up?") was never run, and its honest answer is yes.
    sites: "brief.md:119-122, :1034, :7945-7947; feature-delta 2745-2749, 3213, 3575; ADR-0083"

high:
  - id: R-H1
    issue: >-
      MIS-SCOPED CITATION — "~2.5 s to full guest RAM residency (P13/P14)" is P13's
      ondemand-RESTORE uffd backfill, a banked-probe restore-path property applied to the
      cold-boot path this feature ships. The design's OWN P5 datapoint refutes the
      generalisation: VmRSS at beacon ~270 MB with 128 MiB deliberately touched. SD-4's
      decision is independently justified and unaffected; D-3's urgency framing was inflated.
    sites: "brief.md SD-4 estimation block; feature-delta 3212"
  - id: R-H2
    issue: >-
      EVIDENCE OVERCLAIM — graceful-shutdown row "Proven: P2, both arches". P2 exercised
      guest->host ONLY (findings:357, :2787). The SHUTDOWN command byte is a host->guest
      write on the accepted connection; no probe ever wrote host->guest. The decision stands
      (no acpid + aarch64 PSCI are independent facts; the 2 s escalation bounds the failure)
      but the mechanism is an assumption labelled as proof.
    sites: "feature-delta 3496; ADR-0082 D4"
  - id: R-H3
    issue: >-
      Slice 04 designs the volume path (shared=on, rlimit_fsize, VmPayload.volumes) for BOTH
      shipping arches while P6/shared=on is measured on x86_64 only. Unlike CH exit latency
      and reserve_bytes, this unmeasured premise is labelled NOWHERE.
  - id: R-H4
    issue: >-
      C-1..C-7 slice-text corrections are UNLANDED and unmarked. slice-01:99 and slice-03:68
      still say "cp --reflink=auto" (C-1); slice-01:143 still pins the AC on
      /proc/<vmm-pid>/status for Seccomp (C-5 — fails against correct behaviour); no slice
      mentions image_type (C-2). Only C-5 is relayed in the DISTILL handoff, and DISTILL
      reads the slices.

settled_by_review:
  child_vs_detached_vmm: >-
    It is a CHILD and serve DOES observe mid-run exit. setsid(2) detaches session and
    process-group, NOT parentage (driver.rs:355, :372-377). The "detached therefore
    invisible therefore Bar 2 required" argument does NOT hold. What does make Bar 2
    required: the host-state ensemble drifts mid-run where the exit path is blind — a clone
    leaked by a crash between cleanup steps, a scope/run-dir stranded by a failed stop (the
    VM is then unstoppable until the next serve restart), and SD-2's own clone-leak GC being
    boot-only, so a node whose serve never restarts never sweeps the leak SD-2 quantifies.

bar2_blast_radius:
  - "Hera's 'CREATE NEW: nothing structural — no new Action variant' is FALSIFIED. A Bar-2
     Reconciler mutates only through Actions via the action-shim (ADR-0023); the reap needs
     >=1 new Action variant plus executor surface — the exact variant DD-5 forbade."
  - "New Reconciler machinery: State hydrating cgroup tree + run dirs + staging clones, plus
     the intent-side WorkloadDriver::Vm join; a field-less View per ADR-0079; registration in
     run_server; ESR specs; DST reachability. This IS the 'host/node infrastructure
     reconciler' machinery #197/#198/#199/#234 await — state whether it founds that shared
     model or ships a bespoke fifth."
  - "plan_vm_reap/execute_vm_reap reshape, not loss: reconcile() is the pure diff, Actions
     are the plan, executors the impure half."
  - "Boot ordering changes mechanism: 'reap before adopt_on_restart_recovery' cannot ride a
     broker-driven loop with no bootstrap sweep; needs a synchronous first convergence."
  - "SURVIVING UNCHANGED: reap-not-adopt, the authority rule, budget exemption +
     last_failure_seen_at complement, occurrence-bearing LastTerminated, clone-filename
     attribution, not-Vmm-gated, DD-1's reconciler-site bindings, Hera's four DISTILL scenarios."
  - "New ACs: mid-run drift repair without a serve restart; a live VMM whose allocation row is
     terminal killed at tick N; P2's emission property extended to the new reconciler."

verified_correct:
  - "Hera's DD-1 trap-3 verified line-by-line: workload_lifecycle.rs:1124-1131 / :622-639 /
     :1136-1146 -> Failed{exit_code: Some(0)}. Second emitter :679-707. Ungated
     startup_probe_failed_action service_lifecycle.rs:968-993."
  - "AllocStatusRow.kind is WorkloadKind (observation_store.rs:807/:875/:1178); the two-surface
     join is real."
  - "P1 boot figures, P2 beacon -> 30 s deadline derivation, P4 ~260x + intra-fs constraint,
     P5's three corrections, image_type=raw as v53-scoped (not timeless)."
  - "I-5 single cut; no load-bearing whitepaper citation; no invented issue numbers; no
     test-seam-only premise defended; every new component reachable from run_server/action-shim."
  - "Twelve TransitionReason::Vm* variants well-founded — each has a producer slice and typed
     payload, none duplicates the exec-shaped existing set."
  - "Vmm no-guest-surface boundary coherent and crate-ownership-forced; stop path implementable."

low:
  - "ADR-0082 title and D2 heading still say 'unrepresentable' while the corrected body
     (:229-242) honestly downgrades to 'private fields + one rendering site + a dst-lint
     clause — not a type-level impossibility'. The body governs; the headers overclaim."
```

---

## Wave: DESIGN — adversarial review, iteration 2 (Atlas, fable, 2026-08-11)

All six iteration-1 findings verified FIXED against source, none regressed, blast radius
propagated completely. One NEW high introduced *by* the Bar-2 revisions — it did not exist
at iteration 1 because neither the steady-state tick nor the blank-cell authorisation did.

```yaml
review_id: "arch_rev_2026-08-11_design-wave-adversarial-iter2"
reviewer: "solution-architect-reviewer (Atlas), model=fable"
iteration: 2
approval_status: "conditionally_approved"
critical_issues_count: 0
high_issues_count: 1

iteration_1_findings: { R-C1: fixed, R-H1: fixed, R-H2: fixed, R-H3: fixed, R-H4: fixed, low_adr0082_title: fixed }

new_findings:
  - id: NEW-1
    severity: high
    issue: >-
      Exit-in-flight window. The ordinary VM exit path transits DD-1(b)'s blank cell
      (non-terminal + unsupervised) TRANSIENTLY: the watcher's wait() returns and the
      supervision handle is released on the driver's task, while the terminal row is written
      later by the exit-observer task (§104 pins them as separate tasks). A sweep tick landing
      in that window sees alloc-on-all-three-host-surfaces + row still Running +
      Observed(s)-without-the-alloc, fires diff row 1, and writes
      Terminated/PlatformReclaimed racing the honest exit write on the same LWW key.
      find_prior_alloc_row is used ONLY to resolve workload_id, not as a terminality guard,
      so even LOSING the race does not save it. Consequences are DD-1's own forbidden lies:
      a crash relabelled reclamation escapes the restart budget (crash-looping VM restarts
      budget-free); a COMPLETED Job relabelled reclamation is re-driven — duplicate execution
      of a side-effecting job. No §105a.11 invariant covers it —
      SupervisedVmSurvivesEveryTick requires membership in the supervision set, which this
      alloc has just left.
    recommendation: >-
      (a) release the supervision handle only AFTER the terminal row is written — DD-1(b)'s
      own precondition applied honestly, since while the exit report is in flight the platform
      demonstrably CAN still classify the ending; (b) execute_reclaim_allocation no-ops on a
      terminal re-observed row; (c) read the supervision set BEFORE observe() in
      hydrate_actual so skew fails toward "held" (§105a.2 currently lists the dangerous
      order). Add the window as a fourth ESR invariant.
  - id: NEW-2
    severity: medium
    issue: >-
      plan_reclamation rows 3-4 emit DiscardStrandedArtifacts (whose executor kill_scopes a
      live VMM) with supervision "(not consulted)", falsifying §105a.3's "the ONE
      kill-authorising predicate" claim. Row 3 (terminal) is deliberately right — SD-1's
      unstoppable orphan IS a terminal row with a live VMM — but should be stated as an
      exemption. Row 4 (unknown) is ungrounded: any state where a LIVE VM's intent join
      fails makes it "no entry", and the sweep kills it with no ending authored and no
      supervision check.
  - id: NEW-3
    severity: medium
    issue: >-
      AC 5's byte-unchanged assertion vs a live watcher. The disposal executor structurally
      cannot write a row (no ObservationStore/broker params — good), but its kill_scope on a
      terminal-row VMM that is still supervised fires the live watcher, whose ExitEvent
      advances updated_at at minimum. Shares a root with NEW-1: the supervision handle's
      lifecycle at VMM death / failed stop is unpinned.
  - id: NEW-4
    severity: low
    issue: "brief §106 describes the C-5 slice correction as pending; slice-01:15,180-185 has landed it."
  - id: NEW-5
    severity: low
    issue: "Titan handoff item 6 still says steady-state reclamation is 'Platform Reclamation in DD-4's vocabulary'; superseded by DD-1(b) — it is Artifact Disposal, authors no ending."

fail_safe_trace:
  verdict: "The discriminator does NOT invert."
  detail: >-
    SupervisionSet::Unavailable is #[default] and reclamation_authorised returns false for it;
    hydrate_desired leaves its half at Default, so reading the wrong half yields "nothing
    authorised" — the empty-because-unpopulated case is closed BY THE TYPE, not by review.
    Driver::live_allocations() -> None maps to Unavailable, never "supervises nothing". Both
    Observed(0) cases are known facts about the world: no Vm registry entry means no VmDriver
    exists to hold a handle; at boot the freshly-composed driver's live map is empty by
    construction. Boot and steady state both correct THROUGH THE PREDICATE. The two holes
    found are AROUND it (NEW-1, NEW-2), not in it.

clean_on_recheck:
  - "Hera's two-variant split + payload prohibitions coherent; empty declared delta structurally enforced at the executor (caveat NEW-3)"
  - "The blank cell IS genuinely blank in Titan's table (brief:277-280); her classification does not contradict SD-1's safety property, which is scoped to SUPERVISED"
  - "VmHostState hydration genuinely separable — observe() returns a plain value, plan_reclamation takes NO port; #197 can lift the seam as claimed"
  - "Driver::live_allocations within intake I-2's explicit licence; REUSE->EXTEND reversal recorded honestly; None is the fail-safe default"
  - "30 s sweep: Titan defers to §105a as the single site, no restatement, mechanism identical both ends; three-surface walk cost stated"
  - "Registration/spawn substitution: brief:314-326, C4 edge :760, §105a.7 assert the same property with mutual cross-references; no residual divergence"
  - "Vertical slice: registered unconditionally in run_server WITH a production wake (periodic submission + one boot-drive submission); not a reconciler nothing ticks"
  - "ESR: three invariants + SimVmHostState + SimClock. Typed errors, newtypes, persist-inputs, no whitepaper dependency, no invented issue numbers — clean"

praise: >-
  SupervisionSet is the best type-driven design in this feature — Unavailable as #[default]
  turns the empty-because-unpopulated review hazard into a compile-time property. AC 5 is
  genuinely adversarial: the only assertion that catches an implementation collapsing the two
  Actions into one. And across all three lanes every withdrawn claim (Bar-1, ~2.5 s,
  "Proven: P2") is recorded AS withdrawn in place, with the manner of the error named.
```
