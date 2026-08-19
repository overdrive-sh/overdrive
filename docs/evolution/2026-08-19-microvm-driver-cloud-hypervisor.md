# Evolution — microvm-driver-cloud-hypervisor (GH #42 · ADR-0081/0082/0083)

**Finalized:** 2026-08-19 · **Wave arc:** DISCUSS → SPIKE → DESIGN → DISTILL →
DELIVER (no DEVOPS wave — real-infra envelope inherited from the spike's
two-kernel matrix) · **Branch:** `marcus-sa/microvm-driver-cloud-hypervisor` ·
**PR:** #268 · **Architects:** Titan / Hera / Morgan / Atlas
(nw-*-architect); crafters via nw-software-crafter · **Tracking:** GH
[#42](https://github.com/overdrive-sh/overdrive/issues/42)

> **STATUS — honest, load-bearing.** Implementation complete: **31/31 roadmap
> steps GREEN**, full workspace green, `overdrive serve` + `overdrive deploy`
> boots a real Cloud Hypervisor VM whose guest exit code reaches the operator.
> The production composition root at `lib.rs` now routes through a
> `DriverRegistry` — the feature's pass/fail bar (K4) is met. The
> verification expectation **E06** (`vm-job-deploy-reaches-running`) is
> **satisfied** on the production binary. **Where the evidence stands:** every
> Tier-3 VM boot was proven on the **bare-metal x86_64 KVM box** (Scaleway
> Elastic Metal, AMD EPYC) and the dev-Lima kernel; the **pinned-6.18
> appliance-kernel leg (ADR-0068) and the aarch64 legs are owned by the CI
> LVH kernel-matrix (DWD-10) and are not part of this local signal** — probe
> **P3** (6.18 under CH per arch) and aarch64 `shared=on` remain **unrun**.
> The feature is on-branch (PR #268), **not yet merged**. **Slice 04
> (volumes) was CUT** mid-DELIVER and deferred to overdrive-fs (#97 / #43).

---

## Feature summary

A Cloud Hypervisor **microVM `Driver`** for Overdrive: a workload declared
`[vm]` in a TOML spec boots as a real Cloud Hypervisor VM through the same
`overdrive serve` + `overdrive deploy` verbs a process (`[exec]`) workload
uses, and the platform never reports something about the VM that is not true.
The north star is **exit-status fidelity (K1)**: the operator sees the
**guest's** exit code, never the hypervisor's — the exact lie the reference
implementation (`opencapsule/algiers`) could not avoid, where a guest that
booted, panicked, and shut down cleanly still exited the VMM `0`.

This is a **vertical slice through the production entry points**, not an
isolated mechanism. The load-bearing structural pieces:

- A **`Vmm` port trait** (`overdrive-core`) sits *under* the existing `Driver`
  trait: `VmDriver: Driver` composes over `Arc<dyn Vmm>`, with
  `CloudHypervisorVmm` (production, `overdrive-host`) and `SimVmm`
  (DST, `overdrive-sim`) as the two adapters. The `Vmm` boundary is what makes
  the CH subprocess / vsock / Landlock surface reachable from Tier-1 DST and
  keeps it off every `core`-class dst-lint compile path.
- **`VmConfig` is a value type**, not algiers' stateful config-accumulating
  builder — "boot before configured" is unrepresentable rather than
  runtime-validated (ADR-0082).
- A **`DriverRegistry`** replaces the single hardcoded `Arc<dyn Driver>` at the
  composition root (ADR-0022's pre-committed migration, now earned). Discovery
  → probe → insert happens inside `overdrive serve`'s own boot; spec-parse
  dispatch routes `[vm]` vs `[job]`/`[exec]` (ADR-0083).
- An **in-guest agent, `overdrive-init`** (a new `binary`-class crate, PID 1),
  connects host-ward over **vsock** and emits a `READY` beacon **before any
  guest networking exists**, then a real `WEXITSTATUS`-bearing `EXIT` message.
  `VmDriver::start` does **not** return `Ok` (→ `Running`) until the guest
  signals ready — closing the structural "Running == API returned 2xx" lie the
  action-shim would otherwise inherit for free.
- **`VmReclamation`** — a first-class (Bar-2) reconciler that reclaims leftover
  VM artifacts (cgroup scopes, per-launch rootfs clones, authorship claims),
  built on the **three-ending-classes** model (ADR-0081: guest exit, platform
  reclamation, artifact disposal).
- **Confinement**: the `cloud-hypervisor` process runs non-root
  (uid-drop + `kvm` group), seccomp-filtered (default-on, per-thread),
  Landlock-ruleset-confined, cgroup- and netns-placed — or the workload does
  not run (fail-closed, US-VM-7).

Storage splits by role — `ext4` + `virtio-blk` for the read-mostly rootfs
(per-launch **reflink** clone, ~260× cheaper than a copy), and `virtiofs` for
writable volumes. The volume half (Slice 04) was **cut** — see § Scope changes.

## Business context

`[vm]` is the workload class that needs its own kernel: full kernel-level
isolation, and — the commercial pillar — **CPU hotplug via ACPI**, which is
the genuine Cloud Hypervisor differentiator over Firecracker
([#2609](https://github.com/firecracker-microvm/firecracker/issues/2609) is
parked) and the mechanism GH [#92](https://github.com/overdrive-sh/overdrive/issues/92)
(right-sizing reconciler) depends on. Memory hotplug is **no longer** a
differentiator (Firecracker shipped virtio-mem in 2024-12) — the argument that
survives is "CPU hotplug unblocks #92", not "CH has hotplug." This driver is
the foundation the persistent-microVM arc (#96/#97/#100) builds on.

The feature was scoped hard at PROMOTE (2026-08-10): **boot a VM through
`serve` + `deploy`, nothing else.** Checkpoint/restore, warm pools, the chunk
store, and the persistent guest agent were explicitly banked for #96/#97/#100,
even though the spike measured all of them — the first cut had to close a real
loop, per CLAUDE.md § "Build vertical slices through production entry points."

## Key decisions

### Architecture Decision Records (accepted, permanent)

- **[ADR-0081](../product/architecture/adr-0081-three-ending-classes-platform-reclamation-and-artifact-disposal.md)**
  — three ending classes: a VM allocation ends by **guest exit**, **platform
  reclamation**, or **artifact disposal**; these are distinct and none is
  collapsed into another. Grounds `VmReclamation` and the reclamation-vs-restart
  arbitration.
- **[ADR-0082](../product/architecture/adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md)**
  — the `Vmm` port trait + `VmConfig` anti-corruption **value** family
  (`DiskAttachment`, `MemoryPlan`, `VmRunDir`, `VmConfinement`, …). Amended four
  times during DELIVER: §D4 pinned `DriverError::ResizeUnsupported`; §D7 pinned
  the operator command channel (beacon `EXEC` message); the confinement gap
  (§D2 US-VM-7) and the confined-artifact-access-without-mutating-operator-files
  (4th amendment) were pinned as they were reached.
- **[ADR-0083](../product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md)**
  — the `DriverRegistry` + per-driver allocation payload; §D8 pinned the
  `ServerConfig.vmm_override` test seam (shaped after `mtls_identity_override`,
  a whole-port swap — **not** a whole-subsystem gate).

### Design decisions (DISCUSS `[D1]`–`[D8]`, DESIGN SD/DD/M)

- **`[D2]` Honest `Running`** — gated on a guest-emitted vsock **ready beacon**,
  never on the CH API returning 2xx. The channel *precedes* networking
  (beacon arrives with `guest net ifaces = [lo]`), so the gate never depends on
  the thing it gates.
- **`[D3]` Honest exit** — three distinct situations (clean guest exit,
  guest crash, unreported VMM death), none collapsed; the guest's real
  `WEXITSTATUS` rides the beacon `EXIT` message.
- **`[D4]` Guest agent in slice 1 — YES**, the ~200-line PID-1 `overdrive-init`.
  An agentless VM cannot report guest exit status, and the whole restart/backoff
  model is driven by observed exit status.
- **`[D5]` Rootfs = `ext4` + `virtio-blk`**, per-launch reflink clone. virtiofs
  DAX does not exist on CH, so a virtiofs rootfs buys a supervised daemon and
  FUSE in the hot path for none of its headline benefit.
- **`[D7]` Isolation claim, locked and quoted** — KVM + default-on per-thread
  seccomp + Landlock + cgroup/netns; **no** jailer-equivalent chroot, **no** PID
  namespace. *"VM isolation identical to Firecracker"* is a named forbidden
  sentence; the jailer remainder is [#258](https://github.com/overdrive-sh/overdrive/issues/258).
- **`[D8]` Storage splits by role** — `virtio-blk` rootfs, `virtiofs` volumes.
  This recovered intake decision **I-6**, which had been silently reversed
  during DISCUSS scoping (the rootfs was scoped; volumes fell out by omission).
- **Collapse `MicroVm` + `Vm` into one `vm` driver (intake I-5)** — Overdrive
  will not support full VMs, so two `DriverType` variants dispatching to
  identical code is surface with no purpose. `DriverType::MicroVm` **deleted**;
  `DriverType::Vm` survives; TOML table `[vm]`. Single cut, no compat surface.
- **`JobEnvelope` V1 → V2 rkyv bump** — adding `WorkloadDriver::Vm` shifted the
  `Job` aggregate's archived layout, so the full six-step single-commit
  version-bump procedure ran. `FIXTURE_V1` was **not** touched (see § Lessons).
- **`VmReclamation` at Bar 2** — a **user ruling (2026-08-11)** superseded the
  system-designer's initial converge-on-boot (Bar 1) recommendation: it is a
  registered `Reconciler`, not a boot-time one-shot.
- **Slices are mTLS-exempt and Job-kind-only (`[D6]`)** — stated explicitly; a
  `[vm]`+`[service]` spec is honestly rejected (US-VM-6).

## Work completed

145 commits, DELIVER 2026-08-12 → 2026-08-18. 31 roadmap steps across five
phases (there is no phase 05 — Slice 04 was cut):

- **Phase 01 — Walking skeleton (Slice 01), steps 01-01…01-10.** `Vmm` port +
  `VmConfig` value family; `JobEnvelope` V1→V2 + `WorkloadDriver::Vm`;
  `overdrive-init` guest agent + `vm::beacon` Published Language; real-boot
  fixture provisioning (pinned kernel + ext4 rootfs + static-musl init);
  `reserve_bytes` measured from real-boot `memory.current`; `CloudHypervisorVmm`
  + `SimVmm` with cross-adapter equivalence + FICLONE-per-launch; the `VmDriver`
  (three-way boot race, authorship claim, stop totality); `DriverRegistry` +
  composition root + `[vm]` dispatch + the walking skeleton (the guest boots to
  Running and its exit code is honest — the keystone S-VM-01); `vmm_override`
  fault seam + CgroupAccounting OOM diagnosis; dst-lint clauses + per-thread
  seccomp + `DriverType::MicroVm` deletion.
- **Phase 02 — `VmReclamation` reconciler (SD-1's Bar-2), steps 02-01…02-04.**
  `VmHostState` port + adapters + reconciler skeleton (`plan_reclamation` pure
  diff); authorship-claim lifecycle (release-on-every-arm, hydration read order,
  boot-epoch converge); the two reclamation executors (write-time terminality
  guard + four evaluations + `PlatformReclaimed`); sibling-reconciler guards +
  the four ESR DST invariants.
- **Phase 03 — Boot-failure vocabulary, artifact supply, clone reclamation
  (Slice 02), steps 03-01…03-09.** Typed driver-start failure contract +
  missing-rootfs vertical proof; the named VM boot-failure vocabulary; honest
  **unclassified** fallthrough (verbatim cause, never a raw
  `DriverInternalError` string); `[vm]`+`[service]` rejected at deploy time;
  `[vm]`+`[job]` accepted, scheduled, run to Running on metal; per-allocation VM
  artifacts + unconditional VM composition; capability/clone absence naming the
  cause; per-launch rootfs clones made reclaimable via a durable index.
- **Phase 04 — Stop, restart, VMM death, confinement (Slice 03), steps
  04-01…04-05.** Guest-authoritative `ExitKind`; graceful `overdrive job stop`
  sequencing + bounded grace; restart from a clean rootfs copy; **confinement**
  (Landlock ruleset + uid-drop + rlimits on the spawned process); confinement
  fails closed and adds no operator surface.
- **Phase 06 — `[resources]` sizes the VM (Slice 05), steps 06-01…06-03.** vCPU
  derivation from `cpu_milli` (`max(1, round_up)`, floor 1); guest memory
  matches declared `memory_bytes`; sizing parity + `Driver::resize` honestly
  rejecting with `ResizeUnsupported` citing GH #92.

**Verification:** E06 (`vm-job-deploy-reaches-running`) captured and
**satisfied** against the production `serve` + `deploy` binary (its runner
models the appliance's VM data partition for confined boot).

## Lessons learned

- **The reference implementation's mechanism was wired to nothing.** algiers'
  CH driver had 2 API endpoints and zero production callers —
  `create_virtualizer()` had no caller, so `OPENCAPSULE_VMM=cloud-hypervisor`
  changed nothing. The one defect peer review *caught* (a roadmap step to wire
  the factory) was the one that shipped, because the step was never executed.
  This feature's analogue was the hardcoded composition root; the pass/fail bar
  (K4) was explicitly "is `lib.rs` still `Arc::new(ExecDriver::new(...))`?".
- **`UefiTooBig` is taxonomy, not mechanism.** CH validates the kernel image
  magic, rejects it, then **silently reinterprets `--kernel` as UEFI firmware**
  and reports a 3 MiB size cap — an error that says nothing about the actual
  "wrong image format" cause. The driver must surface a format error naming the
  real problem (`[D5]`). (On x86_64 a distro `bzImage` loads directly; the
  UKI→EFI-zboot→zstd unwrap is aarch64-only.)
- **Version pins rot behind a green board.** The CH pin sat at v46.0 for 14
  months / 7 releases because a buried comment argued "v46 demonstrably has the
  capability we need" — reasoning that read like rigour and was its opposite.
  Every version gate in the repo checks `installed == pinned`; **none** checks
  `pinned == latest`. The sweep that followed found CH stale by 7 releases and
  **wasmtime by 18 major versions**. The right response to an unjustified floor
  is to take latest, not keep the oldest passing build.
- **`image_type=raw` is mandatory from CH v53.** Bare-filesystem images (no
  partition table) auto-detect as `raw` and CH **silently disables sector-0
  writes**; the guest write faults, `panic=1` reboots, and CH cannot reconnect
  a `virtiofsd` that already exited — surfacing two layers from its cause and
  only on `--fs` modes. A driver requirement, not probe hygiene.
- **Reflink is intra-filesystem, and `--reflink=auto` fails silently.** Per-VM
  disk artifacts staged into a tmpfs run dir lose the ~260× reflink win with
  **no error** (coreutils ≥9 defaults to `--reflink=auto`, which degrades to a
  full copy). Stage disk images on the master's filesystem; sockets/logs on
  tmpfs. Assert `FICLONE` works with a real probe, not an fstype string check.
- **Seccomp verifies per-thread.** A correctly confined CH reports `Seccomp: 0`
  on the thread-group leader — the filters live on the worker threads. An AC
  against `/proc/<pid>/status` fails against *correct* behaviour; read
  `/proc/<pid>/task/*/status`. (Caught in the spike, not in Slice 03.)
- **CH's implicit Landlock ruleset omits the vsock UDS.** CH auto-derives rules
  for `--kernel`/`--disk`/`--serial`/`--api-socket` but not for the vsock socket
  it binds itself; the failure is `CreateVsockBackend(UnixBind(EACCES))`, which
  never mentions Landlock. The grant must be the **containing directory**
  (the socket path can't be named — CH validates path existence at parse time,
  before the socket exists), so each VM needs its own socket directory holding
  nothing else.
- **The nested-Apple Lima VM cannot gate microVM boot.** Boots stall ~1-in-3
  (100% on a freshly rebuilt VM); a green run is genuine evidence, a red run is
  uninformative because a real regression and a nested stall are
  indistinguishable. The `infra/metal/` bare-metal x86_64 box is the honest
  Tier-3 boot gate; Lima stays the compile/DST inner loop. Do **not** tune the
  Lima environment — Apple ships nested virt as an undocumented boolean.
- **rkyv enum-root sizing forced FIXTURE_V1 regeneration.** Under rkyv 0.8 an
  enum's archived root is sized to `max(variant sizes)`; adding
  `WorkloadDriver::Vm` grew that footprint and broke direct `from_bytes` decode
  of the pre-existing V1 golden fixture. The resolution (per the version-bump
  procedure) regenerated `FIXTURE_V1` as an **explicit `Envelope::V1(FrozenV1{})`**
  — not via the `latest()`/alias path (which would encode V2 and make the test
  vacuous). This is recorded in project memory as a recurring rkyv trap.

## Blockers encountered (all resolved)

Five DELIVER steps escalated to the architect before proceeding — the "surface
a gap, don't invent API" discipline (CLAUDE.md) firing correctly:

1. **01-02 — rkyv V1→V2 byte preservation.** Adding `WorkloadDriver::Vm` to the
   shared archived enum broke `FIXTURE_V1` decode. Escalated for the
   type-hierarchy / version-bump ruling; resolved by the explicit-V1-fixture
   fork above.
2. **03-01 — classifier VM arm (checkpoint).** `classify_driver_failure`'s VM
   arm could not distinguish kernel-not-found / unclassified / kernel-format
   within its declared scope; the diagnostic data (`VmmExit.stderr_tail`, rootfs
   path, console tail) was not threaded through `DriverError::StartRejected`.
   Recorded as a **checkpoint step**; the work was superseded by the typed
   failure contract in 03-05/03-06/03-02.
3. **03-04 — scheduled-VM descope.** `[vm]`+`[job]` reaches Running (S-VM-39,
   proven on metal); `[vm]`+`[schedule]` (S-VM-40) cannot — no production
   Schedule execution path exists (deferred by ADR-0051 OQ-5 /
   [#166](https://github.com/overdrive-sh/overdrive/issues/166)). Descoped to
   the job case per a scope ruling.
4. **04-04 — confinement never wired.** US-VM-7 confinement (`LandlockRule`,
   `landlock_grant`, setrlimit-before-execve vs `#![forbid(unsafe_code)]`) had
   no production code and no shape. Escalated; the shape was pinned in
   ADR-0082 and the confinement step landed (Landlock ruleset + uid-drop +
   rlimits, fail-closed).
5. **06-03 — resize rejection variant.** No existing `DriverError` variant could
   *honestly* carry "resize unsupported"; escalated to mint
   `DriverError::ResizeUnsupported` in ADR-0082 §D4 rather than flatten a
   capability refusal into `Io`/`StartRejected`.

## Scope changes

- **Slice 04 (volumes) was CUT** (revert `0797140d`, 2026-08-18). A `[[vm.volume]]`
  spec surface and `virtiofs` wiring had begun (step 05-01), but volumes were
  deferred to the **overdrive-fs** work ([#97](https://github.com/overdrive-sh/overdrive/issues/97)
  / [#43](https://github.com/overdrive-sh/overdrive/issues/43)) — a
  vhost-user-fs / chunk-store layer is its own feature. KPIs K8/K9 (output
  fidelity, storage-daemon death) and user stories US-VM-8/9 belong to that
  later slice. The checked-in OpenAPI spec was regenerated to drop the cut
  volume error variants (`e856ec25`).
- **Guardrails 2 & 3 (day-count effort-budget lift triggers) retired** by user
  ruling (2026-08-11), citing CLAUDE.md § "No effort/time budget cuts."

## Deferrals & follow-ups (all tracked)

- [#92](https://github.com/overdrive-sh/overdrive/issues/92) — right-sizing
  reconciler / CPU hotplug (`Driver::resize` currently rejects with
  `ResizeUnsupported` citing this issue).
- [#96](https://github.com/overdrive-sh/overdrive/issues/96) /
  [#97](https://github.com/overdrive-sh/overdrive/issues/97) /
  [#100](https://github.com/overdrive-sh/overdrive/issues/100) — persistent
  microVMs (snapshot/restore, chunk store + vhost-user-fs, persistent guest
  agent). All spike-measured (P8–P14) and banked; **do not design against them
  now.**
- [#43](https://github.com/overdrive-sh/overdrive/issues/43) /
  [#97](https://github.com/overdrive-sh/overdrive/issues/97) — volumes
  (Slice 04, cut).
- [#166](https://github.com/overdrive-sh/overdrive/issues/166) — Schedule
  execution subsystem (the `[vm]`+`[schedule]` case, S-VM-40).
- [#222](https://github.com/overdrive-sh/overdrive/issues/222) — guest-stack
  transparent-mTLS intercept adapter (microVMs terminate TCP in the guest;
  `cgroup_connect4`/sockops are structurally blind).
- [#258](https://github.com/overdrive-sh/overdrive/issues/258) — the jailer
  remainder + `virtiofsd` hardening surface.
- [#259](https://github.com/overdrive-sh/overdrive/issues/259) — OCI/Dockerfile
  → rootfs image factory (the whole of intake I-3's BYO-artifact deferral).
- **Spike probe P3** — the pinned 6.18 appliance kernel under CH, per shipping
  arch — **still not run**; owned by the CI LVH kernel-matrix (DWD-10), not this
  box. aarch64 `shared=on` remains unmeasured (no non-nested Arm hardware).

## Links to permanent artifacts

- **ADRs:** [ADR-0081](../product/architecture/adr-0081-three-ending-classes-platform-reclamation-and-artifact-disposal.md),
  [ADR-0082](../product/architecture/adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md),
  [ADR-0083](../product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md)
  (already in their permanent home).
- **Design (migrated):** `docs/architecture/microvm-driver-cloud-hypervisor/feature-delta.md`
  (DISCUSS + three DESIGN dispatches + two adversarial iterations + DISTILL) and
  `.../spike/` (findings + wave-decisions).
- **Acceptance scenarios (migrated):** `docs/scenarios/microvm-driver-cloud-hypervisor/test-scenarios.md`
  (87 scenarios, S-VM-01…94).
- **Verification:** `verification/expectations/E06-vm-job-deploy-reaches-running/`
  (satisfied on the production binary; kept live in the catalogue).
- **Research:** `docs/research/platform/fly-io-microvm-implementation-research.md`,
  `.../unikraft-microvm-and-dockerfile-reuse-research.md`,
  `.../oci-image-to-microvm-rootfs-research.md`,
  `.../firecracker-vs-cloud-hypervisor.md`,
  `docs/research/testing/trustworthy-tier3-gate-for-microvm-boot-research.md`.
- **Feature workspace (preserved):** `docs/feature/microvm-driver-cloud-hypervisor/`
  — the full history; this document is the summary.
