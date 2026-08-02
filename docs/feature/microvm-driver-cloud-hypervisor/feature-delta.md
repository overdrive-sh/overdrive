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

## Changelog

| Date | Wave | Change |
|---|---|---|
| 2026-08-01 | DISCUSS | Authored from `intake.md` + three research inputs. Extended J-OPS-003 (no new job). Answered the six intake questions as `[D1]`–`[D6]`. Corrected the intake on the Running gate's location (`[G3]`) and surfaced the unrecorded `Job` rkyv envelope cost (`[G4]`). Scope assessed **OVERSIZED**; split applied (Service-kind out; 5 slices). Four blockers surfaced for user approval. |
| 2026-08-02 | DISCUSS (amendment) | **B1–B4 ruled on.** B1 → #257, B4 → #259, B3 → #258 **and rescoped**, B2 **not approved** (#42 untouched; the dependency is dropped with no replacement pointer). Added **`[D7]`** — the locked isolation claim (*KVM + default-on seccomp + Landlock + cgroup/netns confinement; no jailer-equivalent chroot, no PID namespace*) and the six folded hardening items. Placement: items **5–6** → US-VM-1 ACs (inherent to `VmDriver::start`); items **1–3** → new story **US-VM-7** in Slice 03; item **4** (mount-ns) → **#258**, on shape grounds, pending the user amending #258. Added **K7**; added Slice 00 **P5** (confinement composes with a real boot) and extended **P2** (vsock from inside a netns). Rewrote constraint 6, extended constraint 7 (Landlock gives the first genuine CH version-floor reason). Grounded a new finding: `provision_and_inject_netns` gates on mTLS composition, **not driver type**, so a production VM alloc is handed a netns whether or not the driver enters it. **Scope re-assessed: still right-sized — 5 slices, 7 stories**; Slice 00 → 2–3 d, Slice 03 → 4–6 d, Slice 01 unchanged. |
| 2026-08-02 | DISCUSS (review fixes) | Peer review returned `rejected_pending_revisions` with 1 DoR failure + 6 high issues; all fixed, none waived. **US-VM-1's criteria split** — a 5th UAT scenario added (hypervisor containment), UAT-derived ACs given `*(Scenario N)*` back-references, and the four non-UAT-derived engineering items (`lib.rs:1422`, `MicroVm` deletion, rkyv bump, `SimVmm`) moved to a separate **Engineering Constraints** block; recorded as DoR item-5 note. **Three vacuous-pass traps closed:** US-VM-1 item 5's netns assertion now *requires* an mTLS-composed `serve` (the GH #248 / ADR-0074 trap, reproduced deliberately); item 6 now requires the driver to construct the seccomp argument so the mutation target has a real site; US-VM-7 item 1's Landlock denial now names its executor (Slice 00 P5 for denial evidence; the production half is Example 2's spec-derived ruleset; runtime proof is **#258's** EDD item). **US-VM-7's fail-closed producer named** — injected at the `Vmm` port per constraint 1, since the whole test envelope is one Lima kernel. **`<vmm-pid>` resolution named** (allocation `cgroup.procs`), making US-VM-1's cgroup placement a verification prerequisite for US-VM-7. **`[D7]` staged** — until US-VM-7 lands, only *KVM + default-on seccomp + cgroup/netns* may be asserted. US-VM-7's pitch de-overclaimed (confinement is never rendered to Ana; only its absence is) and its "Before" made counterfactual; K7 scoped to mTLS-composed boots with an `n/a` baseline; rlimit ceiling re-anchored to `/proc/self/limits`; Slice 01's cost note argued rather than asserted. **`docs/product/jobs.yaml` corrected** — its J-OPS-003 entry still described the posture as *"the same cgroup treatment ExecDriver gives workloads"* and the security work as *"currently untracked — surfaced as a DISCUSS blocker"* (a forward pointer with no issue number, in a committed SSOT); now cites `[D7]` and #258, and the service-gap reference cites #257. Same corrections applied to `docs/product/journeys/run-a-vm-workload.yaml`. |
| 2026-08-02 | DISCUSS (amendment 2 — **`I-6` recovered**) | **A dropped user input was recovered.** The user's opening message named two reference decisions — *"speak with CH over unix socket and use virtiofsd for storage"*; the first became `I-2`, **the second was dropped by the intake author and never recorded**, after which research recommended `ext4`+`virtio-blk`, DISCUSS scoped that, and the reversal was never surfaced. Recorded as intake **`I-6`** and as **`[D8]`**: **storage splits by ROLE — `virtio-blk` for the rootfs, `virtiofs` for volumes.** Not a compromise — the reference shipped exactly this split (`architecture.md:196`: block for `code.ext4`/`deps.ext4`, virtiofs only for `attach_drive("output", …)`); the apparent research-vs-reference conflict was an artifact of DISCUSS **scoping the rootfs and never scoping volumes at all**. **Two misattributions purged:** CVE-2026-24834 concerns **`virtio-pmem` + DAX**, not virtiofs, and may no longer be cited against virtiofs (constraint 10); and *"the reference used virtiofsd — the option the most experienced team retreated from"* fused two unrelated facts (Kata retreated from `virtio-pmem`; the reference never put its rootfs on virtiofs). `[D5]` rescoped to the **rootfs only** and its *"not virtiofs"* framing removed — it read as a rejection never decided. **New Slice 04 `vm-writes-output-the-operator-can-read` (4–6 d)**, two stories: **US-VM-8** (the `[[vm.volume]]` capability) and **US-VM-9** (the daemon's lifecycle honesty); the former Slice 04 (resources) renumbered to **05**. Volumes placed above resources on outcome impact (5.0 vs 4.0), not effort; the two are mutually independent. Sub-decisions: **`[D8b]` `--memory shared=on` CONDITIONAL** on volume presence (no cost without benefit; one derived field on a `VmConfig` *value*, not two config shapes; regression-safety for slices 01–03) with **Slice 00 P6 measuring the cost**; **`[D8c]` `--cache=never`** (one guest per share, and no CH DAX to cache against); **`[D8d]` `--sandbox=namespace`, fail-closed, NEVER silently downgraded** — correcting the reference's unrecorded `namespace`→`chroot` drift, precedent warning #6's shape; **`[D8e]`** daemon in the allocation's cgroup scope (so `cgroup_kill` reaps it) and netns, with the volume `source` **excluded** from the hypervisor's Landlock ruleset so volumes do not widen `[D7]`. **`[D3]` generalised into system constraint 9** — *a supervised sidecar's death is classified by the WORKLOAD's outcome, never by the sidecar's exit status* — because the reference got it wrong in **both** directions (a clean `virtiofsd` exit read as `VmmError::Crash` and force-killing the VM; a mid-run death not reaching `ExitKind` at all). **#258 reconciled after its two 2026-08-02 amendments:** mount-ns is **out of this feature** (`[B5]` approved) and **virtiofsd hardening is UNCONDITIONAL**, with an explicit lifecycle-vs-posture boundary table in `[D7]`. **`[G4]` rkyv ruling pinned** — `JobEnvelope` **V1→V2**, full six-step single-commit procedure; the hedges in `[G4]`, Handoff item 3 and slice-01's *"(if required)"* removed. Added **K8** (output fidelity) and **K9** (sidecar-death honesty, both directions); 6 new risks; DoR re-run **9/9 across nine stories**. **Scope re-assessed: right-sized, at the upper edge — 6 slices, 9 stories, 5 modules (unchanged), walking skeleton untouched**; a 7th slice or 10th story is the pre-committed trigger to split into two features. |
| 2026-08-02 | DISCUSS (amendment 2 — review fixes) | Peer review of the `I-6`/`[D8]` amendment returned `rejected_pending_revisions` with **1 critical + 5 high**; all fixed, none waived. **CRITICAL — the amendment scoped the HOST half of volumes and never scoped the GUEST half**, which is *structurally the same omission it was correcting* (`I-6` fell out because DISCUSS scoped the rootfs and never scoped volumes), reproduced one level down. `[D8]`'s first draft wrote *"and mounts the share in the guest at `target`"* — passive, **no subject** — silently expanding `[D4]`'s locked four-duty agent scope. Closed by: a **recorded `[D4]` amendment** adding duty **(e) mount each declared volume before exec, and refuse to exec if a required mount fails**; a new **`[D8g]`** scoping the guest side; a fourth domain example and a sixth UAT scenario for the **composite-lie case** (a silently-unmounted share ⇒ the command writes into the discarded rootfs copy, exits 0, and `workload describe` reports `Completed{0}` over an **empty** host directory — every signal individually truthful, the composite false); a `guest-mount-failed` `TransitionReason`; and an extended BYO-artifact contract (the operator's kernel must supply virtiofs). **HIGH ×5:** (1) **`read_only`'s enforcement point was unspecified** while carrying a security framing — pinned **host-side** in `[D8g]`, with the guest-side `-o ro` demoted to an explicitly non-boundary ergonomic guard and a written fallback (strike the framing rather than ship the guest-side version, per `[D7]`'s precedent); (2) **US-VM-9's clean-exit arm was vacuous** — it passes with zero code written, since a do-nothing implementation also yields `Terminated / Completed` and cargo-mutants cannot *insert* a guard — closed by naming the mutation site (the before-vs-during-teardown discriminator), stating that ACs 1–3 are **one** classification not three independent checks, and requiring the Tier-3 case to assert the daemon's exit **was observed while contributing nothing**; K9's paired inverse fixed the same way; (3) **`[D8f]`'s 4–6 d budget omitted five rows** while claiming to be "budgeted honestly" — re-budgeted to **6–9 d** (parse surface, `VmConfig` payload, guest-side mount, host-side `read_only`, a possible second rkyv bump), making Slice 04 the largest slice after the skeleton and the lift question live; (4) **DoR item 5 FAIL on US-VM-8** — two ACs traced to no scenario, the same *technical-AC* defect fixed in US-VM-1 at iteration 1 — closed by adding scenarios 6–7 and an **Engineering Constraints** block with the surface criterion restated as an observable; (5) **DoR item 9 FAIL on US-VM-5** — a bare `PASS` naming no KPI, and none of K1–K9 measured declared-resource fidelity — closed by minting **K10**. Also fixed (medium/low): the split trigger given a **number (>9 d), an owner (DESIGN) and a checkpoint (the DESIGN handoff)** plus a third, wave-unadjustable trigger (>35 d total, or any slice >50% over its band); the **Value-5-vs-lift-authorisation contradiction reconciled** (if Slice 04 lifts, the honest deliverable is a VM class that computes but cannot deliver, and US-VM-8/9 become a hard prerequisite before operator-facing material calls `[vm]`+`[job]` production-ready); `[D8b]`'s load-bearing reason stated as **regression-safety** with P6's measurement given a **two-directional** consequence; K7 scoped to volume-carrying allocations; Slice 00 P6 extended to probe host-side read-only export, the guest mount + its failure shape, and **volume I/O cost under `--cache=never`** (the measurement discipline applied to the rootfs role but skipped for the volume role); P6's gating corrected to include US-VM-5's both-shapes case; Reading checklist `I-1..I-5` → `I-1..I-6`; outcome `(e)` → `(f)`; US-VM-1's rkyv constraint aligned to slice-01's user-ruled wording; the slice-composition gate qualified to *story-bearing* slices. **Totals after fixes: 6 slices, 9 stories, ~22–32 d, DoR 9/9 with seven recorded notes.** |
| 2026-08-02 | DISCUSS (review iteration 2) | Re-review returned **approved** — all 7 blocking and all 7 medium/low iteration-1 issues verified closed, DoR 9/9, JTBD PASS, slice composition PASS, zero anti-patterns. Its 7 new non-blocking findings were then fixed rather than carried: US-VM-1 Scenario 5's second `Then` re-pointed from stop-reach (US-VM-4's scope, Slice 03) to syscall filtering, which also gives AC item 6 a real assertion to trace to; the seccomp mutation obligation re-anchored to an **argv-level** assertion because **CH's `log` mode still installs a filter**, so a `/proc`-only check kills `false` but not `log`; the rlimit comparison anchor changed from `/proc/self/limits` to the `overdrive serve` process (under a Tier-3 harness `self` is the test process); US-VM-7's fail-closed reason stated as a **fifth** `TransitionReason` variant in the Slice 02 shape, reconciled against US-VM-2's "no two share a variant" and K3's "≥ 4 distinct", and added to journey step 3; the `[D7]` staging rule propagated to `jobs.yaml`; the risks table's `lib.rs:1422` reference re-pointed from "an AC" to the Engineering Constraints block; slice-01's Behavior bullet aligned to the corrected AC wording. |
