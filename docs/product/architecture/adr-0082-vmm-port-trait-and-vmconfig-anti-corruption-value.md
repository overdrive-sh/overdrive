# ADR-0082 — The `Vmm` port trait and the `VmConfig` value: the anti-corruption layer against Cloud Hypervisor, with the substrate lies structurally discouraged and lint-enforced

## Status

Accepted. 2026-08-11.
Decision-makers: Morgan (nw-solution-architect, DESIGN wave, third of three).
Mode: propose.
Tags: phase-2, vm-driver, ports-and-adapters, earned-trust, type-driven-design,
application-arch, GH-42.

**Amended 2026-08-17 (third — gap 5 CLOSED: US-VM-7 confinement is built and
applied. `LandlockRule` gains a shape; uid-drop + `setrlimit` + Landlock get an
application mechanism that keeps `overdrive-host` `#![forbid(unsafe_code)]`; and
the confined identity gets a source. See DELIVER step 04-04.).** The 2026-08-12
amendment (§ Status item 5, § D2 method block, § D2.2, § D2.5) deferred
`LandlockRule`, `VmConfig::landlock_rules()`, `VmRunDir::landlock_grant()`, and
the uid/gid-drop + `setrlimit` *application* to Slice 03 (US-VM-7), leaving
`LandlockRule` with "no shape anywhere." This amendment closes that deferral by
formalising **Spike P5's already-proven mechanism** (`spike/findings.md` § P5;
the launch shape at `findings.md:436-441`) into exact signatures — it invents no
posture beyond `[D7]`'s locked claim (KVM + seccomp + Landlock + cgroup/netns;
**no chroot, no PID-ns, no mount-ns** — the jailer remainder is GH #258, out of
scope). No prior decision is reversed.

**(a) `LandlockRule` — the shape (in `crate::vm::config`).**

```rust
/// One `--landlock-rules` grant. `access=rw` is rendered UNCONDITIONALLY and is
/// NOT a field: a read-only rule is insufficient (P5 — a `vsock-only +
/// dir-ro-rule` VM still `EACCES`es), so there is no access parameter to get
/// wrong — the same lever `DiskAttachment` uses for `image_type=raw` (§ D2.1).
/// The grant names a DIRECTORY, never a socket path: CH validates rule paths for
/// existence at config-parse time and the vsock UDS does not exist yet (P5
/// correction 2).
pub struct LandlockRule { path: PathBuf }   // private field

impl LandlockRule {
    /// The complete `--landlock-rules` VALUE: `path=<dir>,access=rw`. One pure
    /// rendering site and a mutation target (a mutant flipping `rw`->`ro` or
    /// dropping `access=` must be killed). The FLAG literal `--landlock-rules`
    /// is rendered separately in `vmm.rs::create` — the sole site the 01-10
    /// dst-lint clause sanctions.
    pub fn to_rule_arg(&self) -> String;
    /// The granted directory — accessor for the dst-lint / behavioural tests.
    pub fn path(&self) -> &Path;
}
```

`LandlockRule` has **no public constructor**: it is built only inside
`crate::vm::config` by `VmRunDir::landlock_grant` (same-module private-field
construction), which is what makes that method the SOLE producer (brief.md
§ 113 — "Landlock rules are never built outside `VmRunDir::landlock_grant`").

**(b) The two producer methods — un-defer them exactly as § D2.2 / § D2 pinned.**

```rust
impl VmRunDir {
    /// The ONE explicit Landlock grant: `access=rw` on the run directory itself.
    /// CH auto-derives rules for `--kernel`/`--disk`/`--serial file=`/
    /// `--api-socket` but NOT the vsock UDS it binds (P5 correction 2); the rule
    /// must be the CONTAINING DIRECTORY (CH rejects a not-yet-existent socket
    /// path at parse time). SD-2 exclusivity is what keeps this grant from
    /// widening. SOLE producer of a `LandlockRule`.
    pub fn landlock_grant(&self) -> LandlockRule;   // LandlockRule { path: self.path().to_path_buf() }
}
impl VmConfig {
    /// The explicit Landlock rules for this launch — today exactly
    /// `vec![self.run_dir.landlock_grant()]`. CH auto-derives the other four
    /// grants, so this list carries ONLY what CH omits (the vsock directory).
    /// `Vec` keeps the signature stable if a future explicit grant is added.
    pub fn landlock_rules(&self) -> Vec<LandlockRule>;
}
```

**(c) The application mechanism — a `prlimit`/`setpriv` argv WRAPPER, NOT
`pre_exec` (this is how `#![forbid(unsafe_code)]` is honoured).** Per-child
uid/gid drop and `setrlimit` are only expressible between `fork` and `exec` —
i.e. in a `Command::pre_exec` closure, which is an `unsafe fn` the crate-wide
`#![forbid(unsafe_code)]` on `overdrive-host` forbids outright. The resolution
is the one P5 proved and the one this codebase already uses for FICLONE:
**encapsulate the unsafe one layer out, in a proven external tool.** The FICLONE
ioctl goes through `rustix`'s safe wrapper (the `vmm.rs` module doc states this
verbatim); the uid-drop + rlimits go through util-linux `prlimit`/`setpriv` as an
argv WRAPPER PREFIX — no `unsafe`, no `nix`/`rustix` fork juggling, exactly P5's
12/12 launch shape (`findings.md:436-441`). The renderer is a new method on the
confinement value:

```rust
impl VmConfinement {
    /// The `prlimit … -- setpriv … --` launch-wrapper PREFIX (argv tokens) that
    /// drops privilege and caps resource limits on the spawned hypervisor with
    /// NO unsafe `pre_exec`. The caller appends the hypervisor binary + its args.
    /// `rlimit_fsize` is passed in — it is a VmConfig derivation
    /// (`VmConfig::rlimit_fsize()` = max(rootfs, guest RAM), C-6) this value does
    /// not hold. One pure rendering site; a mutation target (a mutant dropping
    /// `--no-new-privs`, rendering uid 0, or omitting a wrapper must be killed).
    /// Renders, in order:
    ///   prlimit --fsize=<rlimit_fsize> --nofile=<self.rlimit_nofile> --
    ///   setpriv --reuid=<uid> --regid=<gid>
    ///           --groups=<supplementary,joined> --no-new-privs --
    /// `--groups=<numeric>` (from `VmmIdentity.supplementary`) — NOT
    /// `--init-groups` — because the target uid has no passwd/group DB entry on
    /// the appliance (see (e)); `--init-groups` requires one.
    pub fn launch_wrapper(&self, rlimit_fsize: u64) -> Vec<String>;
}
```

`VmConfinement` **already** carries the uid/gid-drop (`identity: VmmIdentity`),
`rlimit_nofile`, and `seccomp_arg()`; `VmConfig::rlimit_fsize()` (C-6) **already**
exists. This amendment adds ONLY the two Landlock producers and this one
renderer — no other confinement field is minted. `vmm.rs::create` composes the
spawned argv as:

```
launch_wrapper(config.rlimit_fsize())  ++  [hypervisor]  ++  <existing CH args, incl. --seccomp>
                                       ++  ["--landlock"]  ++  ⨁ rule.to_rule_arg() for rule in config.landlock_rules()
```

`--landlock-rules` (the flag literal) and `--seccomp`/`--disk` all live in
`vmm.rs::create` — the one `(file, fn)` pair the 01-10 dst-lint clause sanctions,
already landed and forward-looking — so **brief.md § 113 needs no signature
edit** (the clause guards the FLAG literals in `vmm.rs::create`;
`to_rule_arg`/`seccomp_arg`/`to_disk_arg` render the VALUEs, a distinct concern).

Two application consequences the crafter must discharge (both new when uid-drop
lands; invisible in Slice 01, where CH ran as root):

1. **`argv[0]` becomes `prlimit`, not the hypervisor.** The spawn-time
   `HypervisorAbsent` NotFound detection now observes the WRAPPER chain, not CH
   directly. Resolution (Earned Trust — principle 13): `Vmm::probe()` gains an
   additive scenario proving the confinement toolchain (`prlimit`, `setpriv`)
   resolves on PATH — beside the already-present `--landlock`/LSM/`/dev/kvm`
   scenarios — so at `create()` both CH and the wrapper are known-present
   (wire → probe → use). A confinement-wrapper non-zero exit maps to
   `VmmError::ConfinementUnavailable`, never `HypervisorAbsent`.
2. **Run-directory ownership.** The per-allocation run dir (tmpfs, created by
   root) must be readable/writable by the dropped identity so CH can `bind()` its
   vsock/api sockets there under uid-drop + Landlock. The platform must chown (or
   mode) the run dir to the confined `uid:gid` before spawn. This is a create-path
   responsibility; the crafter determines whether it lands in `vmm.rs::create` or
   in upstream run-dir creation (`vm_driver.rs`) and surfaces a scope addition if
   the latter.

**(d) Fail-closed — the types already exist; this pins the PRODUCER.**
`ConfinementControl` (six variants `Landlock`/`Seccomp`/`UidDrop`/`RlimitFsize`/
`RlimitNofile`/`KvmAccess` — exactly S-VM-51's set), `VmmError::ConfinementUnavailable
{ control, detail }`, `VmStartFailure::ConfinementUnavailable`, and
`TransitionReason::VmConfinementUnavailable` are **already built and wired** (the
total `From<&DriverStartFailure>` conversion carries `control` verbatim; the
`TransitionReason` row is present, emit-status "Slice 03"). Step 04-05 does not
mint them — it builds their PRODUCER. The mapping `vmm.rs::create` applies:

| Applied control fails | `ConfinementControl` |
|---|---|
| `setpriv --reuid/--regid/--groups` (privilege / groups drop) | `UidDrop` |
| `prlimit --fsize` / `RLIMIT_FSIZE` not settable | `RlimitFsize` |
| `prlimit --nofile` / `RLIMIT_NOFILE` not settable | `RlimitNofile` |
| CH rejects `--landlock` / `--landlock-rules` | `Landlock` |
| `/dev/kvm` unreachable under the dropped identity | `KvmAccess` |
| `--seccomp` unavailable | `Seccomp` |

Two fail-closed layers, not one: **(i) boot-time / node-level** — the extended
`probe()` refuses `serve` with `health.startup.refused` when the host cannot
supply `--landlock` / the LSM / `/dev/kvm` / the wrapper toolchain (brief.md
line 626, the uniform Earned-Trust posture); **(ii) per-allocation** —
`VmConfinementUnavailable` for a control that passed the boot probe but fails for
a specific allocation. Step 04-05 drives (ii) through the `SimVmm`
`vmm_override` seam (ADR-0083 § D8) returning `VmmError::ConfinementUnavailable`
directly — no genuinely Landlock-less host exists in the Lima envelope (system
constraint 1). Mutation target: turning the fail-closed arm into
warn-and-continue must be killed.

**(e) The confined identity's source — the Slice-03 "Open DESIGN input",
answered WITHOUT an appliance-image change (no blocker).** slice-03 § Dependencies
asks "which uid/gid the hypervisor drops to, constrained to be resolvable
**without appliance-image changes inside this feature** … else DESIGN returns a
blocker." It is resolvable, so this is the answer, not a blocker:

- **uid/gid**: a fixed, unprivileged, non-root NUMERIC pair the platform reserves,
  supplied by the composition root (`compose_vm_driver` → `VmHostLayout.confinement`)
  as a node-invariant — never core-resident, never operator surface. **No
  `/etc/passwd` / `/etc/group` entry is required**, because
  `setpriv --reuid=<N> --regid=<N>` operates on raw numeric ids — which is
  precisely what satisfies the "no appliance-image change" constraint. Document
  the reserved value as a named const at the compose site.
- **kvm supplementary group**: the `kvm` group gid is base-OS (created by the
  kernel/udev/systemd KVM setup — NOT this feature's appliance), owns `/dev/kvm`
  at `0660 root:kvm`, and is DISCOVERED AT COMPOSE TIME by stat-ing `/dev/kvm`'s
  group owner — the same `metadata.gid()` the `probe_kvm_reachable` path already
  reads. The composition root places it in `VmmIdentity.supplementary`.
- This **replaces** the placeholder currently at `compose_vm_driver`
  (`VmmIdentity { uid: 1000, gid: Gid::new(994), supplementary: vec![] }`), whose
  empty `supplementary` is the specific bug that would deny `/dev/kvm` under
  uid-drop — the comment there already flags it as "an unsourced gap, not a
  considered choice." The uid-drop cannot land until `supplementary` carries the
  kvm gid.
- **The confined uid is SHARED across VMs and does not isolate siblings** —
  Landlock (the per-VM run-dir grant), the per-VM netns and the per-VM cgroup do
  (P5 evidence: a sibling VM's disk is `DENIED errno=13` by Landlock). Per-VM uid
  is NOT US-VM-7 scope; it is a #258 / future hardening. Do not read the shared
  uid as a gap.

Recorded in feature DWD (04-04 gap-5 closure).

**Amended 2026-08-17 (second — the per-launch clone becomes reclaimable;
`RootfsPlan` gains an index link. See ADR-0083's 2026-08-17 (second)
amendment §§ D3f–D3h for the governing decision).** The amendment below moved
the per-launch clone onto an operator-chosen directory, and nothing reclaims a
clone whose allocation ends without `VmDriver::stop`. § D2 gap 3's own pinned
doc comment already says the clone destination "is derived on the **master's
own filesystem** … so a reboot-orphaned clone is attributable (SD-1 / SD-2;
the reap keys off it — ADR-0083 § D7)" — the *attribution* half was true, the
*reap* half stopped being reachable. `RootfsPlan` therefore gains **one field
and one accessor**, and `for_alloc` gains **one parameter**:

```rust
pub struct RootfsPlan {
    master: PathBuf,
    master_bytes: u64,
    clone_dest: PathBuf,   // UNCHANGED — parent(master), FICLONE is intra-filesystem
    index_link: PathBuf,   // NEW — <index_dir>/<same filename as clone_dest>
}

impl RootfsPlan {
    pub fn for_alloc(
        master: PathBuf,
        master_bytes: u64,
        alloc: &AllocationId,
        index_dir: &Path,      // NEW — VmHostLayout::clone_index_dir, itself
    ) -> Self;                 //       `vm::config::clone_index_dir(data_dir)`

    pub fn index_link(&self) -> &Path;   // NEW
    // master() / master_bytes() / clone_dest() unchanged
}
```

`clone_dest` keeps its existing derivation and its existing filename — the
`.overdrive-vm-rootfs-<alloc>.img` convention `RealVmHostState` already parses
is untouched, and the FICLONE intra-filesystem constraint this ADR pinned is
untouched. `index_link` is a **symlink** at the platform-owned index directory
carrying the *same filename*, pointing at `clone_dest`. The lifecycle rule is
an ordering, and it is the whole point: **the link is created before the clone
and removed after the clone**, so at every instant a clone that exists has a
link that exists, and "a clone no sweep can see" is not a reachable state. `VmDriver::stop` and
`cleanup_after_start_failure` remove `clone_dest` first and `index_link`
second; a crash between the two leaves a dangling link that the next
`VmReclamation` sweep disposes idempotently. **No `Vmm` trait method changes,
no `VmConfig` field changes, and `VmConfig.rootfs` still carries the same
`RootfsPlan`** — `Vmm::create` continues to clone `master` → `clone_dest` and
knows nothing about the index. Recorded in feature DWD-26.

**Amended 2026-08-17 (production artifact supply — `VmHostLayout` sheds the
artifact fields; see ADR-0083's 2026-08-17 amendment for the governing
decision).** `VmConfig.kernel` and `VmConfig.rootfs` are now built from the
allocation's own `VmPayload` (ADR-0083 § D3a), not from a node-wide template.
`VmHostLayout` therefore loses exactly two fields — `kernel: KernelImage` and
`rootfs_master: PathBuf` — and keeps only what is genuinely node-invariant
(`cgroup_root`, `run_dir_root`, `arch`, `vcpus`, `confinement`). Its own doc
comment ("Slice 01 ships a single fixed template per node — no per-allocation
BYO kernel/rootfs surface exists yet") is false on this change and is corrected
in the same commit. **`KernelImage::validate` is unchanged and is not
weakened:** its remaining call site is `preflight_kernel`, which already
re-reads the path and re-runs the pure validator *per allocation* immediately
before `Vmm::create`; only the redundant boot-time validation of a node-wide
path disappears with the path itself. The § D2.4 lie-3 / C-7 guarantee — an
unloadable kernel is named as a format error before Cloud Hypervisor sees the
file — holds unchanged, now against the path the operator actually wrote in
`[vm]`. `Vmm::probe` and the wire→probe→use composition invariant are
untouched: the *hypervisor capability* is still proven once at boot, which is
the scope "prove it once, use it many times" was always about. Recorded in
feature DWD-25.

**Amended 2026-08-16 (DELIVER Phase-03 upstream-contract resolution).**
`VmmError` now has exact structured variants for the causes the adapter can
know; `VmProcess` exposes a live `VmmDiagnostics` snapshot backed by the same
bounded console/stderr capture as final `VmmExit.stderr_tail`; and
`VmDriver::start` maps those facts into ADR-0083's typed
`DriverStartFailure`. `Display` text is diagnostic only and is never reparsed.
This closes checkpoint `3222f030` without changing `VmmProbeError` or the
wire→probe→use composition invariant.

**Amended 2026-08-11 (fold-in of prerequisites, same DESIGN pass, user
ruling)** — § D8 is added: a new `CgroupAccounting` port gives the VM
mid-run exit path a post-mortem read of the allocation scope's
`memory.events` `oom_kill` counter, so a cgroup-OOM-killed VM is diagnosed
as `TransitionReason::VmOutOfMemory` (ADR-0083 § D5 row 13) rather than an
undiagnosable `WorkloadCrashedImmediately { signal: 9 }`. This closes
deferral D-3 in its **reduced** form only — a point read, not the live
`memory.events` subscription D-3 names as its mechanism, which stays
deferred, as does the matching gap on `ExecDriver`'s own OOM path. No prior
decision is reversed; § D2.3's `MemoryPlan` / `reserve_bytes` text is
unamended — § D8 only makes that derivation's failure diagnosable. Lands in
Slice 01
(`docs/feature/microvm-driver-cloud-hypervisor/slices/slice-01-vm-job-boots-and-exit-code-is-honest.md`).

**Amended 2026-08-11 (gap-closure, DISTILL-surfaced), same DESIGN pass** —
§ D1 names `SimVmm` as the port's simulation adapter and § D5 (below) names
the five faults it must be able to answer, but neither this ADR nor
`brief.md` § 100 (which separately asserted `SimVmm` is *"the injection
point for Slice 03's fail-closed confinement case"*) ever pinned HOW a real
in-process `overdrive serve` reaches `SimVmm` in place of
`CloudHypervisorVmm`. **ADR-0083 § D8** now pins the seam:
`ServerConfig.vmm_override`, a `#[cfg(feature = "integration-tests")]`-gated
whole-port substitution mirroring the already-shipped
`mtls_identity_override`, ruled explicitly NOT `dataplane_override`-shaped
(that pattern gates off a whole subsystem; this one swaps one port binding
and leaves `probe()` running unconditionally against whatever is bound).
ADR-0083 § D8 also rules that the seam does **not** reach S-VM-67
(`virtiofsd`'s `--sandbox=namespace` check, since no volume information
reaches `VmConfig` or any `Vmm` method) — a boundary stated there, not
resolved here or in ADR-0083 at the time. No decision in this ADR is
reversed; this amendment closes a wiring gap the ADR left open.

**Amended 2026-08-11 (cross-reference update — the open item just above is
now closed), same DESIGN pass** — ADR-0083 § D8 records a user ruling: the
`--sandbox=namespace` posture is verified at the launch-argument
construction layer, mirroring this ADR's own § D2.1 `image_type=raw`
pattern (private fields, one rendering site, a pure unit test on the
rendered value — lint/test-detected, not a type-level impossibility, the
same honesty D2.1 states about itself). **No storage-daemon supervision
port is minted by this feature.** No decision in this ADR changes; only the
cross-reference above is now stale and is corrected by this note.

**Amendment 2026-08-12 (DELIVER 01-01 design-gap closure, GH #42, user
ruling).** DELIVER step 01-01 landed the pure value family (`DiskAttachment`,
`MemoryPlan` + `reserve_bytes`, `KernelImage`, `VmConfinement`; committed
`2636ba1c`) and then **correctly refused to invent API** (CLAUDE.md §
*"Implement to the design"*) for the outer `VmConfig` struct, whose remaining
field types were under-/mis-specified — so `VmConfig`, `VmRunDir` and therefore
the `Vmm` trait (`create(&VmConfig)`) could not compile. Five ratified
resolutions close the gap; the pinned shapes land inline at §§ D1, D2, D2.2
below (which govern), and are summarised here:

1. **`cgroup_scope: CgroupPath` — `CgroupPath` is RELOCATED into
   `overdrive-core`.** It lives today at
   `overdrive_worker::cgroup_manager::CgroupPath` (`adapter-host`), and
   `overdrive-core` (where `VmConfig` lives, § D1 table) cannot depend on
   `overdrive-worker`. The type moves **verbatim** to
   `crates/overdrive-core/src/cgroup.rs` (module `overdrive_core::cgroup`) — it
   is a pure domain identifier, and `adapter-host` was the wrong home; the core
   `CgroupFs` trait already speaks `&Path` (callers pass
   `CgroupPath::resolve(&root)`), so no core→worker cycle exists to break. rkyv
   layout is structural (`struct CgroupPath(String)` — one field), so the
   relocation is byte-compatible for any persisted value.
   `overdrive-worker::cgroup_manager` re-exports it for its existing call sites.
2. **`netns: Option<NetnsName>` — a `NetnsName` newtype is INTRODUCED in
   `overdrive-core`, used at BOTH `AllocationSpec.netns` and `VmConfig.netns`.**
   This **supersedes D-TME-12 / JOIN-1** (which chose `Option<String>` for
   `AllocationSpec.netns`) per explicit user ruling (option (a): newtype both
   sites, accepting the cross-feature blast radius). See § D2 for the pinned
   internal-newtype shape and the supersession record.
3. **`rootfs: RootfsPlan`** and **4. `cmdline: KernelCmdline`** — pinned to
   concrete shapes at § D2 (both resident in `overdrive-core`,
   `crate::vm::config`).
5. **`LandlockRule` + `VmConfig::landlock_rules()` +
   `VmRunDir::landlock_grant()` are DEFERRED to Slice 03 (US-VM-7).**
   `LandlockRule` has no shape anywhere and the slice-01 doc already assigns the
   additive confinement items (Landlock, uid/gid drop, rlimits) to US-VM-7.
   Both methods are removed from the Slice-01 surface (§§ D2, D2.2); Slice 01
   runs Cloud Hypervisor **without `--landlock`/`--landlock-rules`** (so the
   vsock socket needs no grant until Landlock is opted into in Slice 03), and
   the § D2.2 vsock-Landlock necessity argument becomes **operative in Slice
   03, not Slice 01**. Seccomp is unaffected — `VmConfinement::seccomp_arg()`
   stays a Slice-01 deliverable with a landed AC. **CLOSED 2026-08-17 (third
   amendment, top of § Status): `LandlockRule` now has a shape, both producer
   methods and the `VmConfinement::launch_wrapper` application renderer are
   pinned, and the confined-identity source is answered — all built by DELIVER
   step 04-04.**

**D-TME-12 / JOIN-1 supersession — recorded here because the cited canonical
home could not be located.** The "no newtype for the netns field" decision is
cited (in `crates/overdrive-core/src/traits/driver.rs` and `.../vm/config.rs`)
as living in `docs/feature/transparent-mtls-enrollment/design/wave-decisions.md`
— **that path does not exist** (the transparent-mtls-enrollment feature is
archived; the surviving references are in
`docs/architecture/transparent-mtls-enrollment/feature-delta.md` and
`docs/evolution/2026-06-22-transparent-mtls-enrollment.md`, neither of which is
a `wave-decisions.md`, and neither *houses* the newtype-rationale — they only
reference it). Per the fallback discipline, the authoritative supersession is
recorded in **§ D2** below: JOIN-1's "no `NetnsName` newtype" decision is
**REVERSED** for `AllocationSpec.netns` and `VmConfig.netns`, citing GH #42 and
the 2026-08-12 user ruling. JOIN-1's *reasoning* (the value is machine-minted,
bounded, never operator-typed, never persisted) is preserved and shapes the
newtype's completeness level; only its conclusion is reversed.

**DELIVER implementation scope (for the crafter — informational; not a roadmap
edit).** Realising gaps 1 & 2 touches, beyond `crates/overdrive-core/src/vm/config.rs`
(the `VmConfig`/`VmRunDir`/`RootfsPlan`/`KernelCmdline` build):

- **Gap 1 (`CgroupPath` relocation):** move the type verbatim from
  `crates/overdrive-worker/src/cgroup_manager.rs` into a new
  `crates/overdrive-core/src/cgroup.rs` (+ `pub mod cgroup;` in
  `crates/overdrive-core/src/lib.rs`); re-export from
  `overdrive-worker::cgroup_manager` (`pub use overdrive_core::cgroup::{CgroupPath, CgroupPathError};`)
  so existing `overdrive_worker::cgroup_manager::CgroupPath` call sites keep
  resolving.
- **Gap 2 (`NetnsName` introduction):** add `NetnsName` to
  `crates/overdrive-core/src/id.rs`; convert `AllocationSpec.netns`
  (`crates/overdrive-core/src/traits/driver.rs`) to `Option<NetnsName>` and
  rewrite its JOIN-1 docstring to cite this ADR's supersession; mint the value
  in `derive_workload_netns_plan`
  (`crates/overdrive-control-plane/src/veth_provisioner.rs`, the SINGLE mint
  site) via `NetnsName::from_hex4(&slot.to_hex4())`; and thread `NetnsName`
  through the ~9 `AllocationSpec.netns` consumers —
  `crates/overdrive-control-plane/src/{veth_provisioner.rs, reconciler_runtime.rs, action_shim/mod.rs}`,
  `crates/overdrive-core/src/{traits/driver.rs, reconcilers/workload_lifecycle.rs, traits/observation_store.rs}`,
  `crates/overdrive-sim/src/adapters/driver.rs`,
  `crates/overdrive-worker/src/driver.rs`. The crafter must confirm no consumer
  serializes/persists the name (the allocator persists the `NetSlot`, not the
  name); if one does, `NetnsName` needs serde and that is a blocker to surface,
  not to resolve by adding derives silently.

Implements the application-architecture half of
`docs/product/architecture/brief.md` § *System Architecture* → *Cloud Hypervisor
VM driver* (**SD-1 … SD-5**, Titan, 2026-08-10) and § *Domain Model* → *VM
workloads* (**DD-1 … DD-6**, Hera, 2026-08-11). Neither of those sections is
amended; both are consumed.

Ratifies intake **I-2** (a `Vmm` port sits under the `Driver`) and its shape
caveat (*"a `VmConfig` **value** plus a single `Vmm::create(&VmConfig)`, making
'boot before configured' unrepresentable rather than validated"*).

Companion: [ADR-0083](adr-0083-driver-registry-and-per-driver-allocation-payload.md)
(driver dispatch, `AllocationSpec.driver`, the composition gate).

Depends on `.claude/rules/development.md` § "Type-driven design", § "Trait
definitions specify behavior, not just signature", § "Port-trait dependencies",
§ "Errors", § "Persist inputs, not derived state", § "Newtypes — STRICT by
default"; `.claude/rules/testing.md` § "Integration vs unit gating"; CLAUDE.md
§ "Implement to the design — never invent API surface"; ADR-0029
(`overdrive-worker` is the home for driver impls), ADR-0030 §6 (per-driver-class
spec types), ADR-0054 §D5 (`CgroupFs` port), ADR-0068 (pinned appliance kernel).

**No GitHub issue is created and none is invented.** Deferrals are surfaced in
`docs/feature/microvm-driver-cloud-hypervisor/feature-delta.md` § *Wave: DESIGN
— application / component scope* → *Deferrals* for user ruling.

---

## Context

### The problem is not "talk to a hypervisor". It is "translate a substrate that
### lies, at exactly one seam."

The spike (`spike/findings.md`, Cloud Hypervisor v53.0, bare metal x86_64,
`systemd-detect-virt: none`) measured **seven** cases where the substrate
reports success, or reports a failure that names the wrong thing. Titan
tabulated them as SD-5's lie table. Three of them are the ones this ADR must
answer structurally, because all three are **silent** — they produce no error
anywhere:

| Lie | Measured | What it looks like downstream |
|---|---|---|
| `image_type` auto-detect *"disables sector 0 writes"* | P10/P11 | Our bare-filesystem rootfs faults; `panic=1` reboots; diagnosed as a corrupt image, two layers from the cause |
| An unloadable `--kernel` is reinterpreted as **UEFI firmware** and reported as a **3 MiB size cap** (`VmBoot(UefiLoad(UefiTooBig))` for a 23.8 MB kernel) | P1, both arches | The operator reads a firmware size cap and goes looking at file sizes |
| CH auto-derives Landlock rules for `--kernel` / `--disk` / `--serial file=` / `--api-socket` but **not** for the vsock socket it binds itself | P5 | `CreateVsockBackend(UnixBind(EACCES))` — never mentions Landlock |

Hera classified this relationship in DD-6: **Workload Orchestration →
Hypervisor Substrate is an Anti-Corruption Layer, not Conformist**, and named
the `Vmm` port plus the `VmConfig` *value* as the translation layer. C-7 (the
kernel-format variant) is that ACL leaking. This ADR is the ACL, built.

### The reference implementation's shape is a named anti-pattern

Intake I-2's caveat, verbatim: algiers' `Virtualizer` is a stateful
config-accumulating builder (`configure` → `set_boot_source` → `attach_drive` →
`start`) policed at runtime by a `VmState` enum and two `validate_state_*`
functions — *"a hand-rolled state machine re-checking at runtime what a value
type gets for free"*. Its stringly-typed escape hatch (`attach_drive("output",
…)` intercepted on the magic string) is intake precedent warning #5.

The corrective is not "a nicer builder". It is: **one value, one call.**

### What is NOT decided here

Driver dispatch, `AllocationSpec`'s per-driver payload, the composition gate and
the admission rejection are **ADR-0083**. The ending taxonomy, `StoppedBy::
PlatformReclaimed` and the restart-accounting rules are **Hera's DD-1 … DD-5**
(and, if the user rules on H-1, ADR-0081). Checkpoint/restore, persistent
rootfs, warm pools, the chunk store and the guest agent's full protocol are
GH #96 / #97 / #100.

---

## Decision

### D1 — `Vmm` is a four-method port trait in `overdrive-core`; Cloud Hypervisor is its only implementor in scope

```rust
// crates/overdrive-core/src/traits/vmm.rs

pub type Result<T, E = VmmError> = std::result::Result<T, E>;

#[async_trait]
pub trait Vmm: Send + Sync + 'static {
    /// Stable adapter discriminator for structured logs and probe-refusal
    /// events. `"cloud-hypervisor"` for the production adapter, `"sim"` for
    /// the DST binding.
    fn kind(&self) -> &'static str;

    /// Earned-Trust startup probe. Composition-root invariant:
    /// **wire → probe → use** (per `traits/cgroup_fs.rs`'s contract, the
    /// sixth trait instance of the pattern).
    async fn probe(&self) -> std::result::Result<(), VmmProbeError>;

    /// Stage this VM's per-launch rootfs clone and spawn ONE confined
    /// hypervisor process for it.
    async fn create(&self, config: &VmConfig) -> Result<VmProcess>;

    /// Terminate the hypervisor PROCESS: await its exit for `grace`, then
    /// kill it unconditionally.
    ///
    /// **This method does NOT ask the guest to power down.** The guest
    /// request travels on the beacon connection, which `VmDriver` owns —
    /// see § D4 and the correction note below.
    async fn terminate(&self, control: &VmControl, grace: Duration) -> Result<VmTermination>;
}
```

> **Corrected at review iteration 1.** The first draft named this method
> `shutdown` and specified it as *"ask the guest to power down, escalating to an
> unconditional kill"*. That was **unimplementable as pinned**: `VmControl` is
> `{ pid, api_socket }`, the beacon `UnixListener` is bound and accepted by
> `VmDriver` in `overdrive-worker` (§ D2.2), and `CloudHypervisorVmm` lives in
> `overdrive-host` — so the adapter had no handle to the connection the
> mechanism required, and `SimVmm` could not honestly return
> `GuestPoweredOff`, which would have hollowed out the `vmm_equivalence` test
> § D6 makes the enforcement mechanism. The port is re-scoped to the process
> half; the guest half moves to `VmDriver::stop`, where the connection actually
> lives. Per CLAUDE.md § *"Implement to the design"* a crafter must not be left
> to reconcile a contradiction like this.

Four methods, and the count is the decision. Every method the reference
implementation's `Virtualizer` carried beyond these (`configure`,
`set_boot_source`, `attach_drive`) exists only to accumulate state that
`VmConfig` already holds.

**Placement per `.claude/rules/development.md` § "Port-trait dependencies":**

| Item | Crate | Class |
|---|---|---|
| `Vmm` trait, `VmConfig` + every value type below, `VmmError`, `VmmProbeError` | `overdrive-core` | `core` |
| `CloudHypervisorVmm` | `overdrive-host` | `adapter-host` |
| `SimVmm` | `overdrive-sim` | `adapter-sim` |
| `VmDriver` (implements `Driver` over `Arc<dyn Vmm>`) | `overdrive-worker` | `adapter-host` |
| `overdrive-init` (the in-guest PID 1) | `overdrive-init` (**new**) | `binary` |

#### D1.1 — `VmmError` preserves the cause at source; `VmProcess` exposes live diagnostics

The exact adapter-boundary error is:

```rust
pub enum VmmError {
    HypervisorAbsent {
        searched: Vec<String>,
        source: std::io::Error,
    },
    RootfsNotFound {
        path: PathBuf,
        source: std::io::Error,
    },
    ConfinementUnavailable {
        control: ConfinementControl,
        detail: String,
    },
    Create {
        detail: String,
    },
    Io {
        operation: String,
        path: Option<PathBuf>,
        source: std::io::Error,
    },
}
```

`HypervisorAbsent` is used only for a spawn-time `NotFound` and carries every
path the adapter searched. `RootfsNotFound` is used only when the configured
master disappears before or during per-launch staging. A permission error,
short read, clone failure, malformed response, or other I/O failure is not
relabelled as absence; it remains `Io` or `Create` and reaches the explicit
unclassified driver fallback. `ConfinementUnavailable` is the one typed
adapter-local mapping to ADR-0083's confinement cause. Storage-daemon and
guest-mount failures remain `VmDriver` concerns; volume information does not
enter this port.

`VmProcess` gains one required field and one new core value:

```rust
pub struct VmProcess {
    pub control: VmControl,
    pub exit: VmExitWatch,
    pub diagnostics: VmmDiagnostics,
}

/// READ handle. This is the ONLY diagnostics capability `VmProcess` carries,
/// so nothing downstream of `Vmm::create` can mutate the capture.
/// `Clone + Send + Sync`; every clone observes the ONE capture, never a copy.
#[derive(Clone)]
pub struct VmmDiagnostics { /* Arc to the one bounded capture */ }

impl VmmDiagnostics {
    /// The ONLY constructor. Creates one bounded capture and returns its read
    /// handle together with its single write handle. There is deliberately no
    /// `Default` and no reader-only constructor: a reader with no writer is an
    /// orphan capture that would silently report `None` forever.
    pub fn new() -> (Self, VmmDiagnosticsWriter);

    /// Pure snapshot of the guest-console / VMM-stderr tail captured so far.
    pub fn console_tail(&self) -> Option<String>;
}

/// WRITE handle. `Send`, and deliberately NOT `Clone` and NOT `Sync` — exactly
/// one capture task owns the append side of a given process's capture, so the
/// retained order is that task's own sequential order.
pub struct VmmDiagnosticsWriter { /* Arc to the SAME bounded capture */ }

impl VmmDiagnosticsWriter {
    /// Append raw captured bytes. `&self`, so the owning capture task may hold
    /// it behind a shared reference. Infallible: output beyond the bound below
    /// is dropped from the front, never surfaced as an error.
    pub fn append(&self, chunk: &[u8]);
}
```

> **Amendment 2026-08-16 (03-05 implementation gap — the diagnostics WRITE
> capability was unpinned).** The first draft of this subsection gave
> `VmmDiagnostics` private storage and a read-only method, but
> `CloudHypervisorVmm` (`overdrive-host`) and `SimVmm` (`overdrive-sim`) must
> **construct and populate** that capture from another crate — so the type as
> pinned was not implementable. This is the same omission class as the
> 2026-08-14 `VmExitWatch::new` blessing recorded at § D2.4 (a private-field
> return type whose only honest constructor was missing), and it is closed the
> same way: by naming the exact surface rather than leaving a crafter to
> improvise one (CLAUDE.md § *"Implement to the design"*). The reader/writer
> split is the decision — `VmProcess` carries only the reader, so mutation is
> unreachable from `VmDriver`, the § D3 boot race, and the exit watcher, while
> the capture task holds the sole writer. **No generic mutable buffer is
> exposed and no second storage exists.** Additive; no prior decision reversed.

**Bounded-capture semantics (normative).**

- One storage allocation per `VmmDiagnostics::new()` call. Both handles
  reference that one capture: `console_tail()` never copies storage into a
  parallel buffer, and the writer never owns a second one.
- The capture retains at most the last `STDERR_TAIL_LINES` newline-terminated
  lines plus the current unterminated trailing line, and independently at most
  `VMM_CONSOLE_TAIL_MAX_BYTES` (8 KiB) of retained bytes. Whichever bound binds
  first applies, and excess is dropped from the FRONT so the retained text is
  always the most recent output. A single unterminated line therefore cannot
  grow without limit.
- `append` is byte-oriented and makes no framing assumption: a chunk may split a
  line, carry many lines, or carry no newline at all. Line accounting is over
  `b'\n'` only.
- The capture stores BYTES, not `String`. `console_tail()` renders the retained
  bytes with lossy UTF-8 conversion and trims one trailing newline; invalid
  sequences become `U+FFFD`. A front-dropped multi-byte sequence degrades to
  `U+FFFD` — never a panic, never a silent truncation of the whole tail.
- `console_tail()` returns `None` **iff** nothing has been appended, and
  `Some(_)` once any byte has been captured. Two calls with no intervening
  `append` return equal values.

`VmmDiagnostics::console_tail` is **pure-function / return-only**. Its closed
universe is the adapter-owned bounded capture for this one `VmProcess`; it may
read no file, socket, clock, process, or global. `VmmDiagnosticsWriter::append`
is **bounded-change**: its universe is exactly that same capture and its declared
delta is "the retained tail advances", with no other observable effect.

`VmmExit.stderr_tail` is the final snapshot of that same capture — the adapter's
watcher performs its last `append`, then fills the exit value from
`diagnostics.console_tail()` on a reader handle. It is never separately
assembled, so a live deadline snapshot and the final exit tail cannot disagree
about what the process emitted. `CloudHypervisorVmm` and `SimVmm` must satisfy
this live/final coherence in the existing VMM equivalence suite. The field
itself is compile-time-enforced at the port boundary because every successful
`Vmm::create` must return it.

`VmDriver` owns the total structural mapping because it is the boundary from
`Vmm::create -> Result<_, VmmError>` to `Driver::start -> Result<_,
DriverError>`:

| `VmmError` | `DriverStartClass` | Verbatim `DriverStartFailure.detail` |
|---|---|---|
| `HypervisorAbsent { searched, source }` | `Vm(HypervisorAbsent { searched })` | `source.to_string()` |
| `RootfsNotFound { path, source }` | `Vm(RootfsNotFound { path: path.display().to_string() })` | `source.to_string()` |
| `ConfinementUnavailable { control, detail }` | `Vm(ConfinementUnavailable { control, detail: detail.clone() })` | `detail` |
| `Create { detail }` | `Unclassified { driver: DriverType::Vm }` | `detail` |
| `Io { operation, path, source }` | `Unclassified { driver: DriverType::Vm }` | one verbatim diagnostic naming `operation`, `path` when present, and `source` |

No `VmmError::Display` string selects a class. `VmmProbeError` remains a
composition-time `health.startup.refused` cause and never converts into a
per-allocation `DriverError` or `TransitionReason`.

> **Amendment 2026-08-12 (gaps 1 & 2 — two `overdrive-core` residents pinned).**
> "`VmConfig` + every value type below" resolves to concrete `overdrive-core`
> types: `RootfsPlan` / `KernelCmdline` / `VmRunDir` in `crate::vm::config`
> (beside the landed `KernelImage` / `MemoryPlan` / `VmConfinement`). Two field
> types were not previously in `overdrive-core` and now are: **`CgroupPath` is
> relocated** from `overdrive_worker::cgroup_manager` into a new
> `overdrive_core::cgroup` module (worker re-exports it — see § D2), and
> **`NetnsName` is introduced** in `overdrive_core::id` (see § D2). Neither adds
> a dependency; both are required because `VmConfig` and `AllocationSpec` live in
> `overdrive-core` and cannot reach an `adapter-host`-class type.

`CloudHypervisorVmm` lands in `overdrive-host`, not `overdrive-worker`: it is a
**production binding of a core port trait to the host OS**, which is
`overdrive-host`'s stated charter, and it keeps `overdrive-worker` free of
process-spawn machinery for a second workload class. `VmDriver` is
allocation-shaped and therefore belongs beside `ExecDriver` per ADR-0029.

**Required, not defaulted, at the call site.** `VmDriver::new(vmm: Arc<dyn Vmm>,
clock: Arc<dyn Clock>, fs: Arc<dyn CgroupFs>, layout: VmHostLayout)` — every
port is a mandatory constructor parameter. No `with_vmm` builder override: per
§ "Port-trait dependencies", a builder makes the dependency optional and
"optional" means "tests can forget".

### D2 — `VmConfig` is a value whose *derivations* make three substrate lies structurally discouraged and lint-enforced

> **Heading corrected at review iteration 3 (2026-08-11).** This heading and the
> ADR's title previously said *"unrepresentable"*, while the body below
> (*"Precision about what 'unrepresentable' buys here"*) honestly downgrades the
> claim to **private fields + one rendering site + a `dst-lint` clause — not a
> type-level impossibility**. The body governs; the headers were overclaiming a
> guarantee the design does not deliver. Both now say what is delivered.

This is the lever Titan's handoff asked for, and it is worth stating as a rule
rather than as three fixes: **for each lie, the field a crafter could get wrong
does not exist. The correct value is computed from a field that cannot be
omitted.**

```rust
pub struct VmConfig {
    pub alloc:        AllocationId,      // overdrive_core::id
    pub kernel:       KernelImage,       // crate::vm::config — validated magic (lie 3 / C-7)
    pub rootfs:       RootfsPlan,        // crate::vm::config — master + master_bytes + clone dest
    pub cmdline:      KernelCmdline,     // crate::vm::config — platform-derived; NOT operator surface
    pub memory:       MemoryPlan,        // crate::vm::config — guest bytes AND cgroup max (SD-4 / C-3)
    pub vcpus:        NonZeroU8,         // std — derived from cpu_milli, floor 1
    pub run_dir:      VmRunDir,          // crate::vm::config (§ D2.2) — owns every path inside it (SD-2 / C-4)
    pub confinement:  VmConfinement,     // crate::vm::config — identity, seccomp, nofile
    pub netns:        Option<NetnsName>, // overdrive_core::id — from AllocationSpec.netns (both NetnsName)
    pub cgroup_scope: CgroupPath,        // overdrive_core::cgroup — relocated from overdrive-worker (gap 1)
}
```

> **Amendment 2026-08-12 (DELIVER 01-01 design-gap closure, GH #42).** Every
> field above now names a concrete type that resides in (or is relocated to)
> `overdrive-core`, so this struct — and therefore `Vmm::create(&VmConfig)`
> (§ D1) — compiles once the four types below are built. The four resolutions
> that were under-specified at first draft (`CgroupPath`'s home, `NetnsName`'s
> existence and shape, `RootfsPlan`'s shape, `KernelCmdline`'s shape) are pinned
> here so the crafter has zero latitude (CLAUDE.md § *"Implement to the
> design"*). `LandlockRule` is **deferred to Slice 03** — it was never a field
> (see the method block below).

**`cgroup_scope: CgroupPath` — relocated into `overdrive-core` (gap 1).**
`CgroupPath` moves **verbatim** from
`overdrive_worker::cgroup_manager::CgroupPath` to
`crates/overdrive-core/src/cgroup.rs` (module `overdrive_core::cgroup`); its
type, derives (`Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize,
Deserialize, rkyv::{Archive, Serialize, Deserialize}`), `#[serde(try_from =
"String", into = "String")]`, error type `CgroupPathError`, and its
`for_alloc(&AllocationId)` / `as_str()` / `resolve(&Path)` / `Display` /
`FromStr` / `TryFrom` surface are unchanged. `overdrive-worker::cgroup_manager`
re-exports it (`pub use overdrive_core::cgroup::{CgroupPath, CgroupPathError};`)
so its existing call sites keep resolving. `overdrive-core` already carries the
`serde` / `rkyv` derives this needs, and `AllocationId` (used by `for_alloc`) is
already in `overdrive-core::id`; the move introduces no new dependency and — the
rkyv layout being a single `String` field — is byte-compatible for any persisted
`CgroupPath`.

**`netns: Option<NetnsName>` — a new INTERNAL newtype in `overdrive-core`, at
both sites (gap 2). This SUPERSEDES D-TME-12 / JOIN-1.**

```rust
// crates/overdrive-core/src/id.rs — beside the newtype catalogue

/// Machine-minted per-allocation network-namespace NAME (`ovd-ns-<4hex>`).
/// INTERNAL newtype: never operator-typed, never persisted. The value is
/// minted at exactly ONE site — `derive_workload_netns_plan`
/// (`overdrive-control-plane`) — from a validated `NetSlot`; this type makes
/// the shape invariant that derivation previously left implicit explicit.
///
/// Used at BOTH `AllocationSpec.netns` and `VmConfig.netns` (user ruling
/// 2026-08-12, GH #42), reversing JOIN-1's `Option<String>` choice.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NetnsName(String);

impl NetnsName {
    /// The canonical name prefix. `ovd-ns-` (7) + 4 lowercase hex = 11 chars,
    /// ≤ IFNAMSIZ (15) and ≤ NAME_MAX (255) by construction.
    pub const PREFIX: &'static str = "ovd-ns-";

    /// Construct from a 4-char lowercase-hex slot segment (`NetSlot::to_hex4`).
    /// Validates: exactly 4 chars, all `0-9a-f`; the name is `PREFIX + hex`.
    /// This is the ONLY constructor. Core cannot depend on `NetSlot`
    /// (`overdrive-control-plane`), so the constructor takes the already-
    /// rendered hex segment rather than the slot itself.
    pub fn from_hex4(hex: &str) -> Result<Self, NetnsNameError>;

    /// The canonical name string (`ovd-ns-<4hex>`) — for building the
    /// `/var/run/netns/<name>` path and for `setns` targeting.
    pub fn as_str(&self) -> &str;
}
// impl Display for NetnsName { … writes as_str() … }
```

**Completeness level: INTERNAL newtype — a validating constructor +
`Display`/`as_str`, and NO `serde`, NO `rkyv`, NO `FromStr`.** JOIN-1's
still-valid observation drives this: the value has *no operator-typed entry
point*, *no wire/persist boundary*, and *no `FromStr` round-trip to defend*.
`AllocationSpec` derives only `Debug, Clone, PartialEq, Eq` (never serde/rkyv)
and is recomputed each reconcile tick; `VmConfig` is a transient value passed to
`Vmm::create`; the per-host allocator persists the **`NetSlot`**, never the
name; and `adopt_on_restart` recovers a `NetSlot` (not a `NetnsName`) from the
on-disk `/var/run/netns` entries via `strip_prefix`. So no consumer crosses a
wire/persist boundary and the STRICT `FromStr`/serde obligations do not apply —
what *is* enforced is the machine-mint invariant (`from_hex4` validates the 4-hex
shape), which is a strengthening JOIN-1 could not deliver with a bare `String`.
`NetnsNameError` carries the two validation failures (wrong length, non-hex).
`Hash`/`Ord` are derived so any set/map keyed on the name works; they cost
nothing and pre-empt a re-derive.

> **D-TME-12 / JOIN-1 SUPERSEDED (2026-08-12, GH #42, user ruling).** JOIN-1's
> decision *not* to wrap `AllocationSpec.netns` in a newtype (cited in
> `traits/driver.rs` as living in the now-nonexistent
> `docs/feature/transparent-mtls-enrollment/design/wave-decisions.md`) is
> **REVERSED**: both `AllocationSpec.netns` and `VmConfig.netns` become
> `Option<NetnsName>`. This is recorded here because the cited canonical home
> could not be located (see the top-of-file amendment note); JOIN-1's *reasoning*
> is preserved and is exactly why the newtype is INTERNAL (no serde/rkyv/FromStr)
> rather than STRICT. The `traits/driver.rs` field docstring is the crafter's to
> rewrite when it lands gap 2, pointing here.

**`rootfs: RootfsPlan` — pinned shape (gap 3), in `crate::vm::config`.**

```rust
/// The rootfs staging plan: the operator's read-only master image, its size
/// (captured at construction so `VmConfig::rlimit_fsize` stays PURE), and the
/// per-launch clone destination.
pub struct RootfsPlan {
    master: PathBuf,
    master_bytes: u64,
    clone_dest: PathBuf,
}

impl RootfsPlan {
    /// Build the plan for one allocation. `master` is the operator's BYO
    /// rootfs artifact; `master_bytes` is its size, captured HERE by the
    /// caller (the imperative shell does the `stat`). The clone destination is
    /// derived on the **master's own filesystem** (FICLONE is intra-filesystem;
    /// staging into `/run` fails `EXDEV`) with a filename **carrying `alloc`**
    /// so a reboot-orphaned clone is attributable (SD-1 / SD-2; the reap keys
    /// off it — ADR-0083 § D7).
    pub fn for_alloc(master: PathBuf, master_bytes: u64, alloc: &AllocationId) -> Self;

    pub fn master(&self) -> &Path;         // the FICLONE source (read-only)
    pub fn master_bytes(&self) -> u64;     // captured at construction; feeds rlimit_fsize
    pub fn clone_dest(&self) -> &Path;     // the FICLONE target / virtio-blk source
}
```

The exact clone **filename** format is the crafter's (it must sit in `master`'s
own directory and contain `alloc`); the *shape* above is fixed. `master_bytes()`
is what keeps `VmConfig::rlimit_fsize()` pure (see the method block below).

> **Extended by the 2026-08-17 (second) amendment in § Status** — the block
> above is additive-extended, not replaced. `for_alloc` takes a fourth
> parameter (`index_dir: &Path`), the struct gains an `index_link: PathBuf`
> field, and `index_link()` joins the three accessors. `master`,
> `master_bytes`, `clone_dest` and their accessors are unchanged, as is the
> intra-filesystem derivation of `clone_dest` and its filename convention.

**`cmdline: KernelCmdline` — pinned shape (gap 4), in `crate::vm::config`.**

```rust
/// The guest kernel command line. PLATFORM-DERIVED — there is NO operator
/// surface for it (`[vm]` carries kernel/rootfs/command/args, never a
/// cmdline). Constructed by the platform (the VM driver, the imperative shell)
/// from fixed boot parameters — the operator cannot inject kernel parameters.
pub struct KernelCmdline(String);

impl KernelCmdline {
    /// The platform's kernel command line for `arch`. Called by the VM driver,
    /// NEVER from operator input. Renders the fixed platform boot line — the
    /// arch-appropriate `console=…`, `panic=1`, and the virtio-blk `root=`
    /// device for the ext4 rootfs — so the guest kernel boots, mounts the
    /// rootfs and reaches `overdrive-init`. The exact token set is platform
    /// boot policy (the crafter fixes it against a real boot, as with
    /// `reserve_bytes`); the *shape* — a single platform-owned constructor with
    /// no operator input — is what is pinned.
    pub fn platform_default(arch: HostArch) -> Self;

    /// The complete `--cmdline` argument value.
    pub fn as_str(&self) -> &str;
}
```

The operator's *command/args* do **not** ride the cmdline (that would violate
"NOT operator surface"); they reach `overdrive-init` over the **beacon vsock
connection** as a host→guest **`EXEC`** message. The § D7 guest-channel decision
this once punted to is now **made** — see § D7's *Amendment 2026-08-12
(operator-command→guest channel)*, which pins the `EXEC` wire shape (JSON-encoded
argv, spaces/newlines safe), the `READY`→`EXEC`→exec→`EXIT` handshake, and
per-step ownership. Out of `KernelCmdline`'s scope, as before.

There is no `image_type` field, no `memory_max` field and no `rlimit_fsize`
field. Each is a **method**:

> **Amendment 2026-08-12 (gap 5 — Landlock deferred to Slice 03).**
> `VmConfig::landlock_rules(&self) -> Vec<LandlockRule>` is **removed from the
> Slice-01 method set** and, together with the `LandlockRule` type, **deferred
> to Slice 03 (US-VM-7)** — `LandlockRule` has no shape anywhere and the
> slice-01 Dependencies section already assigns the additive confinement items
> (Landlock, uid/gid drop, rlimits) to US-VM-7. Slice 01 launches Cloud
> Hypervisor **without `--landlock`/`--landlock-rules`**, so no run-directory
> grant is needed until Landlock confinement is opted into in Slice 03. Do NOT
> invent `LandlockRule`'s shape now. `rlimit_fsize` (below) is **retained** in
> Slice 01 (C-6). Seccomp is likewise unaffected —
> `VmConfinement::seccomp_arg()` (§ D2.5) is a landed Slice-01 deliverable.

```rust
impl VmConfig {
    /// `max(rootfs image size, guest RAM)` (lie 6 / C-6). Encoded from
    /// Slice 01, BEFORE Slice 04 turns `--memory shared=on` on, because
    /// `shared=on` backs guest RAM with a memfd and a memfd is a *file*
    /// for `RLIMIT_FSIZE`.
    ///
    /// **PURE.** The rootfs size is `self.rootfs.master_bytes()`, captured
    /// at `RootfsPlan` construction by the caller — the same
    /// Functional-Core / Imperative-Shell split § D2.4 applies to
    /// `KernelImage::validate`, where the caller does the `read` and the
    /// validator does not. Without that field this function would be a
    /// `stat(2)` wearing a pure signature, and § 108's "universe ∅"
    /// declaration for it would be false. *(Corrected at review
    /// iteration 1.)*
    pub fn rlimit_fsize(&self) -> u64;
}
```

#### D2.1 — `image_type=raw` (lie 2 / C-2): the value renders its own `--disk` argument

```rust
/// One `--disk` attachment. `image_type=raw` is emitted unconditionally and
/// is NOT a field: CH v53's auto-detect path — which disables sector-0
/// writes and faults our bare-filesystem images — has no representation
/// anywhere in the workspace.
pub struct DiskAttachment { path: PathBuf, readonly: bool }

impl DiskAttachment {
    pub fn new(path: PathBuf, readonly: bool) -> Self;

    /// The complete `--disk` argument value. This is the anti-corruption
    /// translation itself (DD-6), so it lives on the value in `core`, not in
    /// the adapter: exactly one site in the workspace can ever emit a
    /// `--disk` argument, it is pure, and it is a mutation target.
    pub fn to_disk_arg(&self) -> String;   // "path=…,image_type=raw[,readonly=on]"
}
```

`CloudHypervisorVmm` MUST NOT construct a `--disk` value any other way.

**Precision about what "unrepresentable" buys here, corrected at review
iteration 1.** The enforcement is **private fields + one rendering site + a
`dst-lint` clause landing in the same PR** — not a type-level impossibility, and
the ADR must not claim more than it delivers. What is genuinely structural: no
`image_type` value is an input to anything, no operator surface reaches it, and
there is exactly one function in the workspace that can emit a `--disk`
argument. What is convention-plus-lint: that `CloudHypervisorVmm` calls that
function rather than formatting its own string. **The lint clause (§ ADR-0083 /
brief.md § 113) is therefore a Slice 01 deliverable with an acceptance
criterion, not a recommendation** — without it, the claim is convention. The
same correction applies to `MemoryPlan` (private fields, struct-literal-
constructible *within* `overdrive-core`, which is precisely why a
"never struct-literal-constructed" lint clause is needed at all). *(The first
draft also named `LandlockRule` here; per the 2026-08-12 gap-5 ruling
`LandlockRule` and its lint obligation are **deferred to Slice 03** — see the
method-block amendment above and § D2.2.)*

#### D2.2 — the vsock Landlock gap (lie 4 / C-4) and directory exclusivity (SD-2): `VmRunDir` owns every path inside itself

```rust
/// The per-allocation run directory (SD-2 — tmpfs, one per allocation,
/// holding NOTHING else). This type owns every path inside it, which is why
/// SD-2's exclusivity is a structural property rather than a convention.
pub struct VmRunDir(PathBuf);

impl VmRunDir {
    pub fn for_alloc(root: &Path, alloc: &AllocationId) -> Self;
    pub fn path(&self) -> &Path;
    pub fn vsock_socket(&self) -> PathBuf;              // <dir>/vsock       — CH binds
    pub fn beacon_socket(&self, port: VsockPort) -> PathBuf; // <dir>/vsock_<port> — the DRIVER binds
    pub fn api_socket(&self) -> PathBuf;               // <dir>/api
    pub fn console_log(&self) -> PathBuf;              // <dir>/console.log
    // landlock_grant() -> LandlockRule — DEFERRED to Slice 03 (gap 5); see note
}
```

> **Amendment 2026-08-12 (gap 5 — Landlock deferred to Slice 03).**
> `VmRunDir::landlock_grant(&self) -> LandlockRule` is **removed from the
> Slice-01 method set** and deferred, with the `LandlockRule` type, to Slice 03
> (US-VM-7). Slice 01 launches Cloud Hypervisor **without
> `--landlock`/`--landlock-rules`**, so no directory grant is minted this slice.
> The three measured constraints below are the DESIGN rationale for the Slice-03
> grant — **they become operative in Slice 03, not Slice 01** — and are retained
> here so US-VM-7 inherits the pinned reasoning rather than re-deriving it. `C-4`
> is a Slice-03 concern; the earlier "`C-4`'s Landlock grant is *derived*"
> framing on `VmRunDir` above is likewise Slice-03-operative.

Three measured constraints are discharged by this one type **when Landlock is
opted into (Slice 03, US-VM-7)**:

1. **CH does not auto-derive the vsock rule** — so the grant must be explicit,
   and `landlock_grant()` is the only producer.
2. **A read-only rule is insufficient** (`vsock-only+dir-ro-rule` still
   `EACCES`) — so `landlock_grant()` returns `access=rw`, with no parameter to
   get wrong.
3. **The rule cannot name the socket path** — CH validates rule paths for
   existence at config-parse time and the socket does not exist yet
   (`Error validating configuration: Path ".../ch.vsock" provided in
   landlock-rules does not exist`). So the grant is the *containing directory*,
   which is what makes exclusivity a **confinement** property: anything else in
   that directory would be inside the VM's writable reach.

The beacon socket is `<dir>/vsock_<port>` and is bound by **`VmDriver`**, not by
the adapter — CH's guest→host path connects *out* to `<socket_path>_<port>`
(P2: `[HOST t=+0.000s] listening on .../ch.vsock_1234`, then
`accepted guest-initiated connection`). Both sockets sit in the one granted
directory, which is why one grant covers both.

#### D2.3 — `memory.max` cannot equal guest RAM (SD-4 / C-3): `MemoryPlan` has one constructor

```rust
/// Guest RAM and the allocation's cgroup ceiling, together, derived from ONE
/// operator figure. `guest_bytes == cgroup_max_bytes` is not representable:
/// there is no constructor that takes two numbers.
pub struct MemoryPlan { guest_bytes: u64, cgroup_max_bytes: u64 }

impl MemoryPlan {
    /// The ONLY constructor. `declared` is `resources.memory_bytes` — the
    /// figure the operator wrote, and the RAM the guest observes (SD-4
    /// option B). The ceiling adds `reserve_bytes(declared)`.
    pub const fn derive(declared: u64) -> Self;
    pub const fn guest_bytes(&self) -> u64;
    pub const fn cgroup_max_bytes(&self) -> u64;
}

/// The reserve policy. A **function evaluated at start time**, never a
/// persisted field (§ "Persist inputs, not derived state"): a persisted
/// reserve would be a stale cache of this policy, and — per Hera's DD-5 —
/// persisting it would *manufacture* the stored-pair invariant that would
/// then have justified a `VmInstance` aggregate.
pub const fn reserve_bytes(guest_bytes: u64) -> u64;
```

This closes the collision Titan traced to **Slice 01**, not Slice 05:
`CgroupManager::write_resource_limits` writes `memory.max =
resources.memory_bytes` verbatim (`cgroup_manager.rs:346-360`) and `[D1]`
derives guest memory from the same figure. Set both from one number and the
scope is over its limit the moment the guest touches its RAM — and because
`TransitionReason::OutOfMemory` has **no production emit site**
(`transition_reason.rs:56` declares this honestly as `NO — Phase 2`), it
surfaces as `Failed / WorkloadCrashedImmediately { signal: 9 }`,
indistinguishable from `kill -9`.

**The reserve's constant landed measured in DELIVER step 01-05 (commits
`cc4463e7` + `bc1a1157`), not guessed here.** Titan established that RSS
structurally cannot supply it — host page tables for the guest mapping are
charged to the scope via `memory.stat pagetables` and are invisible to RSS —
so the figure was measured, not estimated: a real Cloud Hypervisor boot
(`cloud-hypervisor v53.0`, `--memory …,prefault=on`, which forces full
guest-RAM residency so every reading is the worst-case peak) on the project's
x86_64 KVM metal box (`cargo xtask metal run --`), reading `memory.current` /
`memory.stat pagetables` (never RSS) at settled plateaus across seven guest
sizes (128 MiB → 8192 MiB). The shipped policy is a deliberately conservative
upper bound over every measured point — `reserve_bytes = 8 MiB floor +
guest_bytes / 400` — never the tightest fit; the real measured 2 GiB reserve
is ≈7.04 MiB, superseding the ~5.4 MiB RSS estimate this section originally
carried (the earlier "~11.9 MiB before residency" floor is likewise moot —
`prefault=on` measures with residency forced). Shipping a *guessed* constant
would have been the intake-precedent-#7 "magic version floor" failure; the
measured bound is not one. The full seven-row measurement table lives on the
`reserve_bytes` docstring in `crates/overdrive-core/src/vm/config.rs`.

**Its mutation and property obligations attach at the DELIVER step that
measures it, not at Slice 01** *(corrected at review iteration 1)*. A `todo!()`
body has no behaviour to mutate and no property to assert, so listing
`reserve_bytes` as a Slice 01 mutation target would have been a **vacuously
satisfiable gate**. `MemoryPlan::derive` — which is real from Slice 01 and
carries the `guest != cgroup_max` invariant — is the Slice 01 target;
`reserve_bytes` joins the list in the step that gives it a body.

#### D2.4 — the misleading kernel rejection (lie 3 / C-7): `KernelImage` validates before the hypervisor sees the file, and the validator is pure

```rust
/// A kernel image this hypervisor can actually load. Constructed only by
/// validating the image magic for the host architecture, BEFORE CH sees the
/// path — because CH's rejection path silently reinterprets an unloadable
/// `--kernel` as UEFI firmware and reports a 3 MiB size cap.
pub struct KernelImage { path: PathBuf }

impl KernelImage {
    /// PURE (Functional Core / Imperative Shell): the caller reads the first
    /// `KERNEL_MAGIC_WINDOW` bytes; this function does no I/O.
    ///
    /// - x86_64 accepts a `bzImage` (`HdrS` at 0x202) or a PVH-enabled
    ///   `vmlinux` ELF. A distro `vmlinuz` loads directly (P1, env B).
    /// - aarch64 accepts the raw PE `Image` (`ARM\x64` at 0x38). A distro
    ///   `vmlinuz` is a UKI → EFI-zboot → zstd wrapper and does NOT (P1).
    pub fn validate(path: PathBuf, arch: HostArch, header: &[u8])
        -> std::result::Result<Self, KernelFormatError>;
}
```

`KernelFormatError` carries the **format** problem and the arch. Because the
path can disappear or be replaced after `overdrive serve` constructed the
original `KernelImage`, `VmDriver::start` reopens the configured path, reads the
same bounded magic window, and calls this pure validator for every allocation
immediately before `Vmm::create`. `NotFound` constructs
`DriverStartClass::Vm(VmStartFailure::KernelNotFound { path })`; a format
rejection constructs
`DriverStartClass::Vm(VmStartFailure::KernelFormatUnsupported { path, arch,
detail })`. Both convert structurally to the corresponding
`TransitionReason`; neither invokes CH and neither parses an error string.
This per-allocation check is what makes the post-composition delete/replace
TOCTOU scenarios observable as allocation failures rather than process-startup
refusals.

In `VmKernelFormatUnsupported`, the payload `detail` is the validator's stable
format diagnosis. `AllocStatusRow.detail` remains the separate verbatim
low-level diagnostic channel. The operator never reads `UefiTooBig`; CH's text,
if another race makes it available, may appear only in row `detail`, never in
the variant's meaning.

> **Amendment 2026-08-14 (gap 6 — two 01-06 first-implementor accessors blessed, 01-07 review, user ruling).** Matching gaps 1–5 of the 2026-08-12 amendment, this records two accessors step 01-06 (the `CloudHypervisorVmm` / `SimVmm` adapters) added that the design had not named, both sanctioned here as first-implementor surface (the 01-06 review ACCEPTED both on substance): **`KernelImage::path(&self) -> &Path`** (`crate::vm::config`, `config.rs:201`) — a plain validated-path read, the sibling of `RootfsPlan::master()` (§ D2) and `KernelCmdline::as_str()` (§ D2), which the adapter needs to hand the validated `--kernel` path to the spawn; and **`VmExitWatch::new(oneshot::Receiver<VmmExit>) -> Self`** (§ D3, `vmm.rs:202`) — the only constructor for that private-field return type, structurally forced (the adapter's watcher task must build the value it fills). Both are additive, non-lie-bearing accessors; neither reopens any § D2 substrate-lie decision. Documentation-trail entry only, not a design change.

#### D2.5 — seccomp: three variants, one reachable constructor

```rust
pub struct VmConfinement { identity: VmmIdentity, rlimit_nofile: u64 }

impl VmConfinement {
    pub fn confined(identity: VmmIdentity, rlimit_nofile: u64) -> Self;

    /// The complete `--seccomp` argument value. Same lever as
    /// `DiskAttachment::to_disk_arg`: one pure rendering site in `core`,
    /// and **that site is the mutation target** Slice 01's `[D7]` item 6
    /// requires ("killed by an assertion over the constructed argument").
    /// CH's `log` and `false` modes have no representation anywhere.
    pub const fn seccomp_arg(&self) -> &'static str { "true" }
}
```

> **Corrected at review iteration 1 — the first draft rationalised a weaker
> design.** It kept a three-variant `SeccompMode { Enforce, Log, Off }` on the
> argument that a one-inhabitant type would make Slice 01's mutation-target AC
> **vacuous**. That reasoning is wrong: the AC's stated site is *"an assertion
> over the constructed argument"*, and the **renderer** is a mutation site
> regardless of the enum's cardinality — cargo-mutants substitutes `""` /
> `"xyzzy"` for a `&'static str` body. So the AC is satisfied by
> `seccomp_arg()` **and** `Off`/`Log` become unrepresentable. The first draft
> also named a *different* site in its own mutation-target list
> (`VmConfinement::confined`'s `Enforce` literal) than the AC does, which was
> the tell. Keeping "run the hypervisor with seccomp off" representable in
> `overdrive-core` to preserve a mutation site that already existed elsewhere
> was the one place this ADR abandoned its own governing rule; it no longer
> does.

`VmmIdentity { uid, gid, supplementary: Vec<Gid> }` — settled by P5 and NOT
re-opened: an unprivileged uid in the `kvm` group against `0660 root:kvm`
(`+++ open(/dev/kvm, O_RDWR) OK as spikevmm`). No `0666`, no appliance-image
change; Slice 03's *"open DESIGN input: which uid/gid"* is answered, not a
blocker.

### D3 — `create` returns a process, an exit watch, and a control handle — because `start` races three outcomes

```rust
pub struct VmProcess {
    pub control: VmControl,
    pub exit: VmExitWatch,
    pub diagnostics: VmmDiagnostics,
}
pub struct VmControl { pub pid: u32, pub api_socket: PathBuf }

/// Adapter-agnostic await on the hypervisor process's own ending. Wraps
/// `tokio::sync::oneshot::Receiver<VmmExit>`; the adapter's watcher task
/// fills it. `SimVmm` fills it from an injected script.
///
/// **`&mut self`, NOT `self`** — the receiver must SURVIVE the start race.
/// A by-value `recv` moved the whole watch into the `select!` arm's future,
/// so when the beacon arm won, that future was dropped, the receiver was
/// dropped, the adapter's `send` failed, and **the VMM's exit was never
/// observed** — the allocation would never leave `Running`. It also did not
/// compile: `vm_process.exit.recv()` partially moves `exit`, so the `Ok`
/// arm could not hand the watch to the long-lived watcher, which is exactly
/// what § D3's own race description requires it to do.
/// *(Both defects found at review iteration 1.)*
pub struct VmExitWatch(oneshot::Receiver<VmmExit>);
impl VmExitWatch { pub async fn recv(&mut self) -> Option<VmmExit>; }

/// The HYPERVISOR's ending — never the workload's (`[D3]`).
pub struct VmmExit {
    pub exit_code: Option<i32>,
    pub signal:    Option<u8>,
    pub stderr_tail: Option<String>,   // STDERR_TAIL_LINES, reusing ExecDriver's shape
}

/// The outcome of terminating the hypervisor PROCESS. Deliberately says
/// nothing about the guest — classifying the workload is `[D3]`'s job and
/// runs off the beacon, not off this value.
pub enum VmTermination { ExitedWithinGrace(VmmExit), Killed }
```

The three-way race is `VmDriver::start`'s, and it is **pinned** (SD-3, and
CLAUDE.md § "Implement to the design" — crafters must not improvise it):

```rust
let VmProcess { control, mut exit, diagnostics } =
    vmm.create(&config).await.map_err(/* structural VmmError mapping, D1.1 */)?;
let outcome = tokio::select! {
    biased;
    ready = beacon.accept_ready()          => /* guest signalled READY  → EXEC continuation */,
    ended = exit.recv()                    => /* preserve VmmExit fields → typed StartRejected */,
    ()    = clock.sleep(VM_BOOT_DEADLINE)  => /* snapshot diagnostics    → typed StartRejected */,
};
// On the Ok path `exit` and `diagnostics` are STILL LIVE and move with the
// accepted beacon session into the per-alloc exit watcher.
```

- **`biased;` is load-bearing.** If the beacon and the VMM exit are both ready,
  the beacon wins: a guest that beaconed and then died is a *started* VM whose
  ending belongs to the exit watcher, not to `start`. **This is only meaningful
  because `VmExitWatch::recv` borrows rather than consumes** — the watch must
  outlive the race or the exit watcher receives nothing.
- **The VMM-exit arm consumes the returned value; it never discards it.**
  `Some(VmmExit { exit_code, signal, stderr_tail })` constructs
  `VmStartFailure::GuestExitUnreported { vmm_exit_code: exit_code,
  vmm_signal: signal }`, while `DriverStartFailure.detail` is the captured
  `stderr_tail` or a stable channel-closed diagnostic when the watch yields
  `None`. The operator-visible typed reason therefore carries code/signal and
  the row retains CH's verbatim stderr. Titan flagged CH's failure-to-exit
  *latency* as **unmeasured**; DELIVER measures it, and if it approaches the
  deadline, SD-3 option C (an asynchronous readiness seam) is the named
  re-opening.
- **`VM_BOOT_DEADLINE = 30 s`**, and it is a *policy constant in the driver*,
  not a persisted field and not a magic number. Derivation: the slowest
  measured substrate is **8.7 s** (nested aarch64, module loads + nesting;
  bare metal is **~1.1 s**, 12/12 runs, 16 ms spread), plus guest fsck and the
  three `CONFIG_VSOCKETS=m` module loads — 30 s is ~3.4× the worst observation.
  There is no per-workload input to persist, so § "Persist inputs, not derived
  state" is satisfied trivially; when one appears, the deadline becomes a
  function of it. The deadline arm constructs
  `VmStartFailure::BootDeadlineExceeded { deadline_ms: 30_000,
  console_tail: diagnostics.console_tail() }`; the live diagnostic is never
  reconstructed from timeout text.
- **Every non-`Ok` arm cleans up before returning**: SIGKILL the VMM pid,
  `cgroup.kill` the scope, unlink the run directory and the per-launch clone.
  This is Slice 03's *"no leaked hypervisor processes or rootfs copies"* AC,
  and it must hold on the deadline arm too.

### D4 — the guest request rides the channel the guest already opened, and it is **`VmDriver`'s**, not the port's

**`VmDriver::stop` owns the guest half; `Vmm::terminate` owns the process
half.** The split follows ownership of the connection, which is the constraint
that broke the first draft:

1. **`VmDriver::stop`** writes `SHUTDOWN\n` on the **already-accepted beacon
   connection** it holds for that allocation (the guest opened it; a
   UNIX-domain socket is bidirectional). `overdrive-init` responds with
   `RB_POWER_OFF` → PSCI/ACPI `SYSTEM_OFF` → CH exits 0.

   **What is proven and what is not — corrected at review iteration 3
   (2026-08-11), because the first draft labelled this row *"Proven: P2, both
   arches"* and that is an overclaim.** *Transport and lifetime are proven*: P2
   established that the guest opens **one** connection to CID 2 with no
   handshake and holds it from `READY` through `EXIT n`
   (`spike/findings.md:357`, `separate_reads=2`), and that a guest-side
   `RB_POWER_OFF` → PSCI `SYSTEM_OFF` exits CH `0`. *The host→guest command byte
   is unprobed*: **every vsock probe in the spike exercised guest→host only** —
   `findings.md:2787` records the host→guest direction as explicitly not
   established — and no probe had a guest agent **read** its socket while
   supervising a child process, which is what step 1 requires of
   `overdrive-init`. **The decision is unaffected**, because the two facts that
   reject the alternative (no `acpid` in a ~200-line PID 1; aarch64 uses PSCI,
   not an ACPI button) are independent of this mechanism, and step 2's
   `VM_SHUTDOWN_REQUEST_DEADLINE` escalation bounds the failure to 2 s of extra
   latency on a path that lands `Terminated / Stopped { by: Operator }` either
   way. **First exercised by the Slice-03 Tier-3 stop AC**, which is therefore
   the mechanism's first real evidence rather than a regression guard on
   something already measured.
2. **`VmDriver::stop`** then calls `Vmm::terminate(&control, grace)`, which
   awaits the VMM's exit for `grace` and `SIGKILL`s it on expiry.
3. **`VmDriver::stop`** finishes with `cgroup.kill` on the scope and removal of
   the run directory and the per-launch clone.

**The window the first draft did not name, and it is real.** A stop can arrive
**before the guest has beaconed** — between `Vmm::create` and `accept_ready`.
There is no connection then, so there is nothing to write `SHUTDOWN` to. The
pinned behaviour: **skip step 1 and go straight to `Vmm::terminate`.** A guest
that has not yet signalled readiness has not started the operator's command, so
there is nothing to shut down gracefully, and the allocation still lands
`Terminated / Stopped { by: Operator }` — never a crash. `VmDriver::stop` is
therefore total over every point in the start path, which is the property the
first draft assumed rather than stated.

**Because `Driver::stop` has no `grace` parameter** (`traits/driver.rs:418`:
`async fn stop(&self, handle: &AllocationHandle) -> Result<(), DriverError>`),
`VmDriver` sources both bounds from its own policy constants, pinned on the same
terms as `VM_BOOT_DEADLINE`:

| Constant | Value | Derivation |
|---|---|---|
| `VM_SHUTDOWN_REQUEST_DEADLINE` | **2 s** | Bounds **step 1's write**. A guest not reading its socket must not block `stop`; expiry falls straight through to step 2. Well above the measured `READY`→`EXIT` round-trip (~0.3 s, both arches, P2) and well below the grace |
| `VM_STOP_GRACE` | **10 s** | Bounds **step 2**. Covers guest fs sync + `RB_POWER_OFF` → PSCI/ACPI `SYSTEM_OFF` → CH exit. Mirrors `ExecDriver`'s `DEFAULT_STOP_GRACE` role rather than inventing a new policy shape |

**Without the step-1 deadline this ADR's own claim — *"an unresponsive guest
still lands `Terminated / Stopped { by: Operator }`"* — has no mechanism**: the
bundled `shutdown` bounded the whole interaction, and splitting it bounded only
the process half. *(Found at review iteration 2.)*

`VmDriver` holds the session alongside its per-allocation live state:

```rust
struct LiveVm {
    control: VmControl,
    beacon:  Option<BeaconSession>,   // None until the guest dials
    scope:   CgroupPath,
    run_dir: VmRunDir,
    rootfs:  RootfsPlan,
}
```

`Option` is the type-level statement of the pre-beacon window: step 1 is
`if let Some(session) = &live.beacon`, so *"there is no connection yet"* is not
a case to remember — it is the only other inhabitant.

**Enforcement, and the relocation created a gap here.** `vmm_equivalence.rs`
drives the **`Vmm` port**, so it structurally cannot reach the relocated guest
half. `VmDriver::stop`'s edge cases — pre-beacon stop, unresponsive guest,
already-dead VMM, double stop — are therefore asserted by a **`VmDriver`-level
acceptance case against `SimVmm`**, named here so the move does not quietly shed
the enforcement it was partly justified on.

> **Amendment 2026-08-14 (01-07 review, item 1 — the post-stop `status()` contract).** `Driver::status`'s post-stop contract (`traits/driver.rs`: after `stop() → Ok(())`, `status() → Err(NotFound)`) binds `VmDriver`. The shipped `VmDriver::stop` left the live-map entry `Live` and so returned `Running` from `status()` until the exit watcher happened to fire — a real violation. `VmDriver::stop` now transitions the entry `Live → EndingInFlight` under the same lock it extracts the live state with (control/beacon/scope/run_dir/rootfs), **before** the SHUTDOWN/terminate/teardown steps above; `VmDriver::status` maps `EndingInFlight → NotFound`, so the contract holds synchronously. The claim is **retained** (`live_allocations()` reports `EndingInFlight`), so `VmReclamation`'s `EndingInFlightIsNeverReclaimed` (brief § 105a.11) holds across the stop→terminal-row window — a full removal (the `ExecDriver` shape) would drop the claim and let a reclamation author a competing `PlatformReclaimed` ending. This mints **no** `Driver`-trait carve-out: `EndingInFlight` is an in-flight authorship claim, not the terminal-state memory the contract forbids. The transition (row **3b**), its race-safety with the watcher's transition 3, and the operator-stop authorship on the stop path (transition 6) are pinned in brief § 105a.3. Lands as the 01-07 review-remediation; DWD-20.

**Rejected: CH's `PUT /api/v1/vm.power-button` over `--api-socket`.** It is the
obvious mechanism and it is wrong here for a measured reason: our guest is a
~200-line static PID 1 with no `acpid`, so on x86_64 the ACPI button event has
no in-guest consumer, and on aarch64 CH uses PSCI rather than an ACPI button at
all. Reusing the vsock channel needs no new transport, works identically on both
shipping arches, and the connection is **measured open** for exactly the window
in which a stop can arrive (the guest holds it from `READY` until `EXIT n`, P2).
What is *not* measured is the write **into** it — see the correction in step 1
above; the channel's availability is evidence, the host→guest command is an
assumption bounded by `VM_SHUTDOWN_REQUEST_DEADLINE`.

`--api-socket` is nevertheless **kept in `VmConfig`** — it costs one socket in
an already-granted directory, CH auto-derives its Landlock rule, and it is the
substrate `Driver::resize` will need when GH #92 lands. It is not depended on
by any path in this feature.

An unresponsive guest still lands `Terminated / Stopped { by: Operator }`, never
a crash (Slice 03 AC) — `VmTermination::Killed` is an outcome of the
*mechanism*, not a classification of the *workload*, which is why the enum
deliberately carries no guest-shaped variant.

### D5 — `Vmm::probe()` is the sixth trait instance of the Earned-Trust pattern, and its fault-injection scenarios are enumerated

Reuse row 3 of Titan's gate: five port traits already carry `probe()`
(`CgroupFs` at `traits/cgroup_fs.rs:235`, `MtlsResolve` at
`traits/mtls_resolve.rs:179`, `MtlsEnforcement` at
`traits/mtls_enforcement.rs:588`, plus `ViewStore` and `JournalStore` in
`overdrive-control-plane`). `Vmm::probe()` copies `CgroupFs`'s contract shape
verbatim — *"Called once at composition-root startup … failure causes the
process to refuse to start with a structured `health.startup.refused` event."*

Per CLAUDE.md principle 13, the probe is a **first-class design
responsibility** and its fault-injection scenarios are specified, not left to
DELIVER:

| # | Scenario the probe must survive | Substrate lie it closes | `VmmProbeError` variant |
|---|---|---|---|
| 1 | The VM image directory cannot `FICLONE` (`EOPNOTSUPP` on ext4, `EXDEV` across filesystems) | Lie 1 — `cp --reflink=auto` degrades to a **full copy with no error**; P4 measured 0.015 s / +0 MiB versus 3.970 s / +4096 MiB | `ReflinkUnsupported { dir, fstype, source }` |
| 2 | The installed `cloud-hypervisor` has no `--landlock` | Lie 4 — a CH built without it silently runs unconfined | `LandlockFlagAbsent { binary, version }` |
| 3 | The host kernel does not expose the Landlock LSM (`/sys/kernel/security/lsm`) | Lie 4, host half | `LandlockLsmAbsent { lsms }` |
| 4 | `/dev/kvm` is not openable **under the target identity** | Lie 7 — `0660 root:kvm`; a uid-dropped VMM reaches it only via group membership | `KvmUnreachable { uid, gid, mode, source }` |
| 5 | The run-directory root is absent or unwritable — an executed `mkdir` → `bind` → `unlink` round-trip on a probe-scoped subdirectory | SD-2 — the run directory must be creatable and bindable, since the vsock and beacon sockets both land in it | `RunDirUnusable { root, source }` |

**How these five scenarios reach `SimVmm` inside a real `overdrive serve`
(added 2026-08-11, gap-closure amendment).** This table specifies WHICH
faults `SimVmm` must be able to answer; it does not specify how `SimVmm`
gets composed in place of `CloudHypervisorVmm` for a Tier-3 test exercising
S-VM-13 (scenario 1, non-reflink staging) or S-VM-51 (confinement, via
scenarios 2–4). That wiring is **ADR-0083 § D8** — `ServerConfig
.vmm_override`, a whole-port substitution seam gated behind
`#[cfg(feature = "integration-tests")]`, ruled there as the port-boundary
pattern (`Sim*` swapped in for exactly one port) rather than a
`dataplane_override`-shaped subsystem gate. `Vmm::create`'s confinement
failures and `virtiofsd`'s own sandbox check are two different things —
the former is reachable through this table's scenarios and § D8's seam;
the latter (S-VM-67) is not, because no volume information reaches
`VmConfig` or any `Vmm` method — see ADR-0083 § D8's closing section for
that boundary stated in full, including the 2026-08-11 user ruling that
closes it (argv-layer assertion, no storage-daemon supervision port
minted).

Scenario 1 is an **executed `FICLONE`**, not an fstype string comparison —
`infra/metal/provision.sh:419-430` already does exactly this
(`cp --reflink=always` against a real 8 MiB file) and is the pattern to reuse.
A fstype check is precisely the "asking the substrate to describe itself"
failure Earned Trust exists to refuse.

> **Corrected at review iteration 1.** Scenario 5 originally read *"absent,
> unwritable, or **not tmpfs**"* with an `fstype` field — a filesystem-type
> string comparison, i.e. exactly the failure the paragraph above condemns two
> lines later. The contradiction is removed by dropping the tmpfs assertion
> from the probe, and it costs nothing: **the reap does not need the run
> directory's fstype, only its absence-after-reboot** (SD-2 calls it an *epoch
> marker*), and absence is directly observable by the reap itself. What the
> probe genuinely must establish is that the root is creatable and bindable,
> which is executable. If a future need for the tmpfs property arises, it is a
> `/proc/self/mountinfo` read — a substrate self-description that would have to
> be justified on its own terms, not smuggled in under an executed probe.

**Self-application (principle 13, recursively).** The boot probe can go stale —
a remount, a package upgrade or a different staging path invalidates it. So two
lies keep **per-launch** enforcement as well: `image_type=raw` is structural in
`DiskAttachment` (D2.1) and the clone uses the **`FICLONE` ioctl directly**, not
`cp`. The ioctl either succeeds or returns a typed errno; there is no
`--reflink=auto` path to silently degrade, and no coreutils-version dependency.
The probe is the gate; the per-launch mechanism is the proof the gate is still
honest.

### D6 — the trait's contract is behaviour, and a two-adapter equivalence test enforces it

Per § "Trait definitions specify behavior, not just signature", every method
carries preconditions, postconditions, edge cases and observable invariants in
its rustdoc. The load-bearing edge cases, pinned so they cannot be
"interpreted":

| Method | Edge case | Contract |
|---|---|---|
| `create` | `config.rootfs` clone destination already exists (a crashed prior launch) | Replace it. The clone is per-launch and carries no state a restart may inherit — Slice 03's *"a restarted VM boots from an unmodified copy"* |
| `create` | The spawn fails after the clone succeeded | The adapter removes the clone before returning `Err`. No partial artifact escapes a failed `create` |
| `create` | `config.netns` is `None` | The VMM runs in the host netns. Not an error — Job-kind VMs need no tap, and an mTLS-uncomposed boot never supplies a netns |
| `terminate` | The VMM is already gone | `Ok(VmTermination::Killed)`. Idempotent; a second stop is not an error |
| `terminate` | `grace == 0` | Kill immediately, with no await |
| `probe` | Called twice | Idempotent; leaves no probe-scoped residue (`CgroupFs::probe`'s stated postcondition) |

**Enforcement:** `crates/overdrive-host/tests/integration/vmm_equivalence.rs`
drives `CloudHypervisorVmm` and `SimVmm` through the same call sequence and
asserts observable equivalence at every step, gated behind `integration-tests`.
The contract is the spec; the equivalence test is the enforcement; the
per-adapter implementation is the consequence. Without it, "production and sim
observe the same behaviour" is a slogan.

### D7 — the guest side: a new `binary`-class crate and a Published Language

`overdrive-init` is a new crate (`crate_class = "binary"`), built static for
`{x86_64,aarch64}-unknown-linux-musl`, shipped as an artifact the operator bakes
into their BYO rootfs. Its five duties are `[D4]`'s and are not expanded here.

The wire protocol is a **Published Language** (Hera's DD-6), pinned as a pure
module `overdrive_core::vm::beacon` that both sides depend on, so host and guest
cannot drift:

```
guest → host   "READY <k=v>...\n"      exactly once, before exec
guest → host   "EXIT <status>\n"       exactly once, after waitpid
host  → guest  "SHUTDOWN\n"            at most once (D4)
                EOF                     terminates the session
```

Line-oriented ASCII, **two distinct reads** for READY and EXIT — the ordering is
*observed*, not parsed out of one blob (P2: `separate_reads=2`).

Two measured guest requirements ride along and are appliance/rootfs contract,
not code: **`/dev/console` must exist statically in the ext4 image** (the kernel
opens it as fd 0/1/2 for init before devtmpfs is up; without it there is no
console output at all, which reads as a hang), and `CONFIG_VSOCKETS=m` means
three modules load in dependency order before the beacon — **built-in is
strongly preferable** in the appliance kernel and removes a rootfs↔kernel
coupling from the one path the Running gate rides on.

> **Amendment 2026-08-14 (guest vsock provisioning — who loads the modules, GH #42, DISTILL DWD-21).**
> The paragraph above named the choice ("built-in in the appliance kernel,
> *or* three modules load before the beacon") but never pinned the loader or
> the staging path, and step 01-08's walking skeleton hit exactly that gap:
> on the stock `CONFIG_VSOCKETS=m` kernel the 01-04 fixture stages, the
> guest's `socket(AF_VSOCK, …)` fails `EAFNOSUPPORT` and never reaches the
> beacon (`vm_walking_skeleton.rs` module doc; console captured 2026-08-14).
> Pinned here, matching the spike's proven-12/12 mechanism
> (`spike-scratch/increment-a/build.sh:76-84`,
> `probe/src/bin/guest_init.rs:82-231`; findings § P2):
>
> 1. **`overdrive-init` is the loader.** Before `connect_beacon`, it
>    `finit_module(2)`-loads the three vsock modules — `vsock.ko`,
>    `vmw_vsock_virtio_transport_common.ko`, `vmw_vsock_virtio_transport.ko`,
>    **in that dependency order** — from a well-known in-guest directory (the
>    spike's `/modules/`; the fixture and `overdrive-init` share one pinned
>    path const so they cannot drift), tolerating "already loaded / built-in"
>    (`EEXIST`) as success and "module file absent" as skip.
>    `nix::kmod::finit_module` (nix "kmod" feature) keeps the crate's
>    `#![forbid(unsafe_code)]` intact. A genuine load failure surfaces as a
>    typed `InitError` variant logged to `/dev/console` (development.md §
>    "Errors"), never swallowed; a still-unavailable `AF_VSOCK` remains the
>    existing typed `EAFNOSUPPORT` surface, never a silent hang.
> 2. **The rootfs contract stages the `.ko`s.** The 01-04 fixture (and any
>    non-`vsock=y` rootfs) stages those three modules, zstd-decompressed
>    (`finit_module` takes uncompressed ELF), from
>    `/lib/modules/$(uname -r)/kernel/net/vmw_vsock/*.ko.zst` — the **same
>    `uname -r`** the staged kernel came from, so modules and kernel cannot
>    version-skew. This is appliance/rootfs contract, like `/dev/console`
>    above.
> 3. **No-op on a vsock=y appliance kernel.** ADR-0068 §4 (amendment
>    2026-08-14) requires the production appliance kernel to build vsock in;
>    such an image stages **no** modules, so step 1's load finds nothing to do
>    and `connect_beacon` proceeds directly. The SAME `overdrive-init` binary
>    is correct on both kernels with no `#[cfg(test)]` branch and no
>    test-only parameter — kernel-config-variance resilience and the
>    documented `[D2]` fallback, NOT production shaped by simulation
>    (development.md § "Production code is not shaped by simulation": the
>    audit checklist passes — an absent module dir is one `read_dir`→`ENOENT`,
>    no production degradation, self-explaining from this contract).
>
> **Scope:** a **01-08 review-remediation** touching
> `crates/overdrive-init/src/main.rs` (01-03's file) and
> `crates/overdrive-testing/src/vm_fixture.rs` (01-04's file) — both
> cross-step, added to step 01-08's `implementation_scope`. Unblocks the
> `#[ignore]`d S-VM-01/02/05/74; enables authoring S-VM-15 (needs a live
> beacon). `Vmm` / `DriverRegistry` / the allocation payload are untouched,
> so **ADR-0083 is unaffected**.

**`ExitKind::CleanExit` for a VM means "the guest agent reported a clean exit",
never "the platform verified the workload succeeded"** (Hera's DD-4). No
artifact may state or imply otherwise.

> **Amendment 2026-08-12 (operator-command→guest channel — rides the beacon, GH #42, user ruling).**
> § D2's gap 4 punted *"how `overdrive-init` learns the command/args"* to *"a § D7
> guest-channel decision,"* and the first cut of § D7 never made it — a hole that
> blocks S-VM-01's walking skeleton (the guest cannot run a command it was never
> told). The user ruled 2026-08-12: **the operator's command + args reach
> `overdrive-init` over the beacon vsock connection as a host→guest message, NOT
> the kernel cmdline** (§ D2.4's `KernelCmdline` stays platform-only). Pinned here
> so the crafter has zero latitude (CLAUDE.md § "Implement to the design").
>
> **1 — the message: `EXEC`, a fourth `BeaconMessage` variant carrying a JSON-encoded argv.**
>
> ```
> host → guest   "EXEC <json-argv>\n"    exactly once, after READY is accepted, before the guest execs
> ```
>
> where `<json-argv>` is `serde_json::to_string(&argv)` over `argv: Vec<String>`
> — e.g. `EXEC ["/usr/bin/render","--out","/tmp/a b"]`. The variant is
> `BeaconMessage::Exec { argv: Vec<String> }`, `argv[0]` the program and `argv`
> the full vector.
>
> **Why JSON, not a space-split token line.** The operator's argv is arbitrary
> strings that may contain spaces AND newlines. The landed `BeaconMessage::from_str`
> is space-delimited (`line.split(' ')`) and the module's framing is
> one-message-per-line (`\n` terminator): a raw argument with a space would split
> into phantom tokens, one with a newline would break the line framing outright.
> JSON string escaping closes both — `serde_json` escapes every embedded `"`, `\`
> and control character, crucially `\n`→`\n` and `\r`→`\r` (two chars each), so the
> encoded array is **single-line by construction** and the newline framing is
> preserved. `serde_json` is a standard workspace crate (development.md § "Use
> standard crates" — do not hand-roll an escape scheme), is `core`-class-safe (no
> tokio/rand/net), and a `Vec<String>` round-trips deterministically. Rejected:
> length-prefixed binary payload (breaks the one-line/`read_line` discipline the
> whole module is built on) and per-arg base64 (works, but JSON is more legible on
> the wire and idiomatic here).
>
> **Parse/format, pinned exactly (extends the landed `Display`/`FromStr`
> round-trip + structured `BeaconParseError`):**
> - `Display`: `write!(f, "EXEC {}", serde_json::to_string(argv).map_err(|_| fmt::Error)?)`.
>   Serialising a `Vec<String>` is total; the `fmt::Error` arm is the unreachable
>   sentinel (§ "logically unreachable" adapted to `Display`), never `.expect()`
>   (this is a `core` library crate — development.md § "No `.expect()` in
>   production library code").
> - `FromStr`: the `EXEC` arm decodes the **entire post-kind remainder as one JSON
>   array** — it does NOT space-tokenize the payload (the split that is correct for
>   `READY`/`EXIT`/`SHUTDOWN` is wrong here). `READY`/`EXIT`/`SHUTDOWN` parsing is
>   unchanged.
> - Two new `BeaconParseError` variants, both carrying only `Clone + PartialEq +
>   Eq` data so the enum's existing derives survive — a `serde_json::Error` is none
>   of those, so do **not** `#[source]`-embed it: `EmptyArgv { raw: String }` (a
>   `[]` argv has no `argv[0]` to exec — invalid by construction) and
>   `MalformedArgv { raw: String, detail: String }` where `detail` is the serde
>   error's `Display`.
> - **`Copy` is dropped from `BeaconMessage`'s derive list** (`Vec<String>` is not
>   `Copy`); it keeps `Debug, Clone, PartialEq, Eq`. Call sites relying on `Copy`
>   move/clone — a mechanical consequence, pinned so it is not dodged with a
>   `Copy`-preserving shape.
> - `EXEC` carries **argv only** — no env, no cwd, no PATH policy; none is on the
>   `[vm]` operator surface (§ D1), so none is invented. The guest execs `argv[0]`
>   as given (absolute path, mirroring `ExecDriver`).
>
> **2 — the handshake sequence (Candidate A) and what `READY` means.** Unchanged
> from the landed `beacon.rs` `READY` semantics — this amendment pins the sequence
> *around* them, it does not move them:
>
> ```
> 1. guest dials CID 2 : BEACON_VSOCK_PORT, connects (one connection, no handshake — P2)
> 2. guest → host  READY pid=1 port=1234   exactly once — wins the § D3 biased race → Running
> 3. host  → guest  EXEC ["…"]              exactly once, on the § D3 Ok continuation (below)
> 4. guest forks; child execs argv[0] with argv, inheriting /dev/console stdio (§ D7)
> 5. guest → host  EXIT <status>            exactly once, after waitpid
>    host → guest  SHUTDOWN                 at most once, any time after step 3 succeeds (§ D4)
>                  EOF                       terminates the session
> ```
>
> `READY` means **"the guest agent (PID 1) is up and has opened the beacon; the
> operator's command has NOT yet started."** (Candidate A, not B: under B `READY`
> fires only *after* the command starts, so a slow fork/exec would trip
> `VM_BOOT_DEADLINE` on a healthy boot and Running would stop meaning "guest up.")
> This is exactly what § D3's beacon arm and § D4's pre-beacon-window text already
> assume.
>
> **The `EXEC` write is folded into § D3's Ok continuation and gates Running —
> reconciling S-VM-01 with the pre-command window.** On the `accept_ready` win,
> before the select is allowed to yield `Ok(handle)`:
> - VmDriver writes `EXEC <argv>` on the just-accepted session. **Write ok** →
>   store the session in § D4's `LiveVm.beacon` (now `Some`), move the still-live
>   `exit` watch into the exit watcher (§ D3), return `Ok(handle)` → **Running**.
>   **Write err** → construct `VmStartFailure::GuestCommandDispatchFailed {
>   detail }`, with the same verbatim diagnostic in `DriverStartFailure.detail`,
>   then treat it as `StartRejected` exactly like the other non-Ok arms (§ D3's
>   "every non-`Ok` arm cleans up"): SIGKILL the VMM, `cgroup.kill`, unlink dir +
>   clone, release the supervision claim → **Failed**, no leak. It is a known
>   typed cause, not the unclassified fallback.
> - The honest consequence: **Running ⟹ READY arrived AND `EXEC` was delivered** —
>   Running now means "guest up and command dispatched," strictly ≥ the ready
>   beacon, so S-VM-01's *"Running reached no earlier than the ready beacon"* holds
>   and is in fact tightened. This does not reopen § D3's race (the three arms are
>   unchanged); it extends only the Ok *continuation*.
> - **§ D4's `LiveVm.beacon` becomes `Some` only after the `EXEC` write succeeds.**
>   This *strengthens* D4's stop totality and removes any host-side `EXEC`/`SHUTDOWN`
>   race: a stop in the [READY-consumed → EXEC-sent] window sees `beacon == None`
>   and, per D4, skips the `SHUTDOWN` write and goes straight to `Vmm::terminate`
>   (lands `Terminated / Stopped { by: Operator }`). The guest therefore **never
>   observes `SHUTDOWN` before `EXEC`** — its read model stays: block for exactly
>   one `EXEC`, then (D4 duties) supervise the child while watching for at most one
>   `SHUTDOWN`. The pre-beacon window (§ D4, before step 2) is unchanged.
>
> **3 — how `overdrive-init` consumes it.** After sending `READY`, the guest blocks
> on `BufRead::read_line` for exactly one host→guest message; on `EXEC { argv }` it
> forks and the child execs `argv[0]` with `argv` (replacing the loud placeholder
> the 01-03 draft ships), inheriting `/dev/console` as the child's stdio, while
> PID 1 `waitpid`s and reports the real `WEXITSTATUS` as `EXIT <status>` (§ D3),
> then reads at most one further `SHUTDOWN` (§ D4). No second parser — `EXEC` is
> decoded through the same `overdrive_core::vm::beacon` module both sides already
> share (Hera's DD-6).
>
> **Honesty — the host→guest write is unprobed, same status as `SHUTDOWN`.** Every
> vsock probe in the spike exercised guest→host only (`spike/findings.md:2787`; the
> § D4 step-1 correction, review iteration 3). `EXEC` rides the same unprobed
> host→guest direction as `SHUTDOWN`; the connection's *availability* is measured
> (the guest holds it open from `READY`), the host→guest *write* is an assumption.
> Its **first real evidence is S-VM-01's Tier-3 walking skeleton (step 01-08)** — a
> real `overdrive deploy` whose guest runs the operator's command — not a
> regression guard on something already measured. Do not label this row "proven."
>
> **Ownership — every piece has a landing step, so no deferral is left unowned**
> (this closes the 01-03 review's *"unowned deferral"*):
>
> | Piece | Lands in | Surface |
> |---|---|---|
> | `EXEC` variant + JSON codec + two `BeaconParseError` variants + `Copy` drop | **01-03** (owns `overdrive-core/src/vm/beacon.rs`) — in the 01-03 review-remediation, since the guest consumer is already 01-03's | `overdrive_core::vm::beacon` |
> | `overdrive-init` reads `EXEC`, forks, child execs `argv[0]` | **01-03** (owns `overdrive-init/src/main.rs`; criterion "execs the operator's command") | guest |
> | VmDriver **writes** `EXEC` on the § D3 Ok continuation, gates Running, stores the session only after | **01-07** (owns `VmDriver` + the three-way race + the `LiveVm` session) | host |
> | Operator `command`/`args` **source** → `VmDriver::start` (via `AllocationSpec.command`/`args`, `[G5]`, threaded through `DriverInput::Vm`), driven by a real `[vm]+[job]` deploy | **01-08** (owns spec-parse dispatch + composition root + the S-VM-01 walking skeleton) | host wiring |
>
> **Boundary — stdio forwarding is a *separate* channel and is NOT a second gap
> for this one.** `overdrive-init`'s duty (c) *"forwards stdio"* (§ [D4] scope)
> rides `/dev/console` → CH stdout/stderr → `VmmExit.stderr_tail` (§ D3), a data
> path already pinned by § D7's `/dev/console` requirement and entirely independent
> of the beacon *control* channel (vsock CID 2). S-VM-01's gating ACs assert on the
> **exit code** (carried by `EXIT <status>`, already landed), not on host-captured
> stdout, so the command channel is unblocked without any stdio framing. If a
> future AC ever needs *structured, per-stream* host capture of the child's stdout
> distinct from init's own console logging, that framing is a separate,
> currently-unpinned concern — it does not block Slice 01 and is deliberately not
> decided here.

> **Amendment 2026-08-14 (01-07 review, item 2 — the `EXEC` write is not yet wired; ownership reaffirmed as 01-07).** The ownership table above assigns *"VmDriver **writes** `EXEC` on the § D3 Ok continuation, gates Running, stores the session only after"* to **step 01-07** — but 01-07's four roadmap criteria never encoded it, and the shipped `VmDriver::start` beacon-win arm (`crates/overdrive-worker/src/vm_driver.rs`, commit `e4f6602e`) stores the accepted session and spawns the exit watcher **without** writing `EXEC`. So the "folded into § D3's Ok continuation and gates Running" text above is **not yet implemented** (not wrong): a real 01-08 boot would leave the guest — which already blocks on exactly one `EXEC`, landed 01-03 (`BeaconMessage::Exec` + its JSON codec in `overdrive_core::vm::beacon`) — waiting forever. It lands as the **01-07 review-remediation** (a fifth 01-07 criterion; roadmap step 01-07, DWD-20), **not** reassigned to 01-08: `vm_driver.rs` is 01-07's scope, not 01-08's, and this table's own row already owns the write there. The mechanism is 01-07 (VmDriver writes `EXEC <serde_json argv>` from `spec.command`/`spec.args` via the landed `BeaconMessage::Exec` `Display`; a write error is treated as `StartRejected` with the same cleanup + claim release as every other non-Ok race arm; `LiveVm.beacon` becomes `Some` only after the write succeeds). The operator command **source** (`[vm]+[job]` → `AllocationSpec.command`/`args`, threaded through `DriverInput::Vm`) and the real-guest **proof** remain **01-08** (S-VM-01), exactly as the table's last two rows assign — `EXEC`'s first real evidence is still the 01-08 Tier-3 walking skeleton, unchanged.

### D8 — the cgroup-OOM diagnosis gap (deferral D-3, reduced form): a new `CgroupAccounting` port, read once at exit time

*(Added 2026-08-11, folding in prerequisite D-3 per user ruling — anything
this feature needs to work properly lands now, not as a deferral. Scope is
deliberately narrow: a **post-mortem read**, not the live `memory.events`
subscription D-3 names as its mechanism. That subscription — and the
matching gap on `ExecDriver`'s own OOM path — stays deferred; this closes
the VM mid-run path only.)*

§ D2.3 states the failure mode this closes, verbatim: `MemoryPlan::derive`
correctly makes `guest_bytes == cgroup_max_bytes` unrepresentable, but when
`reserve_bytes` is wrong — and it ships as a RED scaffold (§ D2.3,
Consequences) — the resulting cgroup OOM *"surfaces as `Failed /
WorkloadCrashedImmediately { signal: 9 }`, indistinguishable from `kill
-9`"*. That sentence is a bug report against this ADR's own design, not
just against deferral D-3 in the abstract, and it is worse than a rare edge
case: `brief.md` § SD-4 already establishes that a VM's declared RAM is a
**standing claim** whose host-resident share trends toward the declared
figure over the run, so a wrong `reserve_bytes` is an **expected** overrun,
not a corner case.

**The read.** `CgroupAccounting`, a new port beside `CgroupFs`
(`traits/cgroup_fs.rs`) — **not** an extension of it; see "Why not widen
`CgroupFs`" below.

```rust
// crates/overdrive-core/src/traits/cgroup_accounting.rs
#[async_trait]
pub trait CgroupAccounting: Send + Sync + 'static {
    /// Read the `oom_kill` counter out of the `memory.events`
    /// pseudo-file at `memory_events_path` (the caller resolves and
    /// joins the full path -- same convention as `CgroupFs::write`,
    /// which also takes a fully-resolved file path).
    ///
    /// # Postconditions on Ok
    /// Returns the current value of the `oom_kill` key, parsed from
    /// `key value\n` lines. `0` is a real, positive fact ("the kernel
    /// has never OOM-killed a process in this scope"), not a default.
    ///
    /// # Errors
    /// - `Io` -- the substrate `read` failed (`NotFound`,
    ///   `PermissionDenied`, ...). At the ONE call site this port is
    ///   used from (immediately after the exit-watcher's `wait`/
    ///   `recv()` resolves, before any teardown -- see below),
    ///   `NotFound` is an anomaly, not a benign race.
    /// - `Malformed` -- the content parsed as valid UTF-8 but had no
    ///   `oom_kill` line. cgroup v2 guarantees the key when the
    ///   `memory` controller is enabled; its absence means the
    ///   controller was never enabled for this scope, or the path is
    ///   not `memory.events` at all.
    async fn oom_kill_count(&self, memory_events_path: &Path)
        -> Result<u64, CgroupAccountingError>;

    async fn probe(&self) -> Result<(), CgroupAccountingProbeError>;
    fn kind(&self) -> &'static str;
}

#[derive(Debug, thiserror::Error)]
pub enum CgroupAccountingError {
    #[error("cgroup accounting read failed: {source}")]
    Io { #[source] source: std::io::Error },
    #[error("memory.events has no parseable 'oom_kill' line: {raw:?}")]
    Malformed { raw: String },
}
```

**Call site — the mid-run exit watcher, not the boot race.** § D3's
three-way `start` race is unaffected: an OOM during the 30 s boot window
still falls through to `VmBootDeadlineExceeded` / the existing
`StartRejected` path (ADR-0083 § D5 rows 4 and 7), and closing that corner
is out of this fold-in's scope — the measured floors in § D2.3 put the risk
almost entirely in steady-state residency, not boot. The read belongs in
the **per-alloc exit watcher** that `exit` (§ D3's `VmExitWatch`) is moved
into once the beacon has won the race — the Slice-01-built, VM-shaped
analogue of `ExecDriver`'s `spawn_exit_watcher`. Immediately after that
watcher's `wait`/`recv()` resolves and **before any teardown** (so the
scope's `memory.events` is still readable), and only on the "no agent EXIT
report, VMM died" branch, it calls `cgroup_accounting.oom_kill_count(&scope
.resolve(&cgroup_root).join("memory.events"))`.

**Threading the fact through the SHARED `ExitEvent` — one additive field,
one additive precedence check.** `ExitEvent` (`traits/driver.rs`) gains:

```rust
pub struct ExitEvent {
    pub alloc: AllocationId,
    pub kind: ExitKind,
    pub intentional_stop: bool,
    pub stderr_tail: Option<String>,
    /// Set only when the driver observed, immediately after exit and
    /// before any teardown, that the allocation's cgroup scope had a
    /// nonzero `oom_kill` counter. `ExecDriver` never sets this (its
    /// own OOM diagnosis is the unreduced half of D-3, still
    /// deferred). `None` means "not observed to be OOM" -- it does
    /// NOT mean "confirmed not OOM": a read error also yields `None`,
    /// per this fold-in's best-effort scope.
    pub oom: Option<OomFacts>,
}

pub struct OomFacts { pub limit_bytes: u64, pub oom_kill_count: u64 }
```

`limit_bytes` costs no I/O — it is `MemoryPlan::cgroup_max_bytes()`, already
held by the driver from `start`.

`overdrive-control-plane`'s `worker::exit_observer::handle_exit_event`
(Slice 01's Dependencies list previously read *"exit observer … reused
unchanged"* — corrected in the slice by this fold-in, since this is no
longer true) gains one precedence check ahead of its existing `Crashed →
WorkloadCrashedImmediately` mapping:

```
ExitKind::Crashed { .. } if event.oom.is_some_and(|o| o.oom_kill_count > 0)
    → TransitionReason::VmOutOfMemory { limit_bytes, oom_kill_count }   // ADR-0083 § D5 row 13
ExitKind::Crashed { exit_code, signal }
    → TransitionReason::WorkloadCrashedImmediately { .. }               // unchanged default
```

No `DriverType` branch is needed — `ExecDriver` never populates `oom`, so
every Exec crash falls through unchanged. `VmOutOfMemory`'s `StoppedBy`
disposition is `Process` (an ordinary crash), the same as
`WorkloadCrashedImmediately` — **it is NOT `PlatformReclaimed`**: DD-1's
third ending class is about the platform losing supervision, never about
*why* a supervised VM died. So `VmOutOfMemory` consumes restart budget
exactly as any other crash does, no exemption; `is_restartable` /
`is_intentionally_stopped` need no change (ADR-0083's reuse rows 1–2
already cover this).

**Composition — gated with `Vmm`, not unconditional like `VmHostState`.**
`VmHostState` (`brief.md` § 105a.2) is composed unconditionally because
reclamation must clean up a node that has since **uninstalled** CH.
`CgroupAccounting` has no such job — it is consulted only by the VM
exit-watcher, which exists only when `VmDriver` is composed. It rides SD-5's
same composition gate (ADR-0083 § D2): probed alongside `Vmm`, refusing the
node on the same substrate-lie / capability-absence split. `VmDriver::new`
gains a required parameter, extending the constructor already shown in
ADR-0083 § D2:

```rust
// was: VmDriver::new(Arc::new(vmm), clock, fs, vm_layout)
VmDriver::new(Arc::new(vmm), clock, fs, cgroup_accounting, vm_layout)
```

**Why not widen `CgroupFs`.** ADR-0083 § A8 already rejected this once, for
`VmHostState`'s need, on the trait's own contract — *"deliberately
write-only … the read side is unexposed by design"*. That reasoning is not
scoped to `VmHostState`'s enumeration; it is stated as a property of the
trait itself. This need is narrower than `VmHostState`'s (one already-known
cgroupfs path, not a node-wide enumeration spanning non-cgroupfs surfaces)
and was tempting to fold in as "just one more read method" — but doing so
would make `CgroupFs`'s write-only contract mean one thing at one call site
and another thing at the next. Reusing A8's verdict keeps it meaning one
thing everywhere it's read.

**Why not ride `VmHostState::observe()`.** Wrong cadence and wrong crate.
`observe()` is a whole-node, periodic-tick snapshot consumed by the
control-plane's `VmReclamation` reconciler (`brief.md` § 105a); this read
must happen **synchronously, once, immediately after `child.wait()`/
`recv()` resolves**, in `overdrive-worker`, before any teardown a later
reclamation tick might perform. Routing it through the reconciler's cadence
would both mistime the read (the scope may be gone by the next tick — and
per § 105a.3, `VmReclamation` is *authorised* to remove a scope with no
live supervision, which this exit-watcher currently is) and reach across a
crate boundary `VmHostState`'s one consumer was never meant to cross.

**Probe (Earned Trust, CLAUDE.md principle 13).**
`RealCgroupAccounting::probe()` reads `<cgroup_root>/memory.events` at the
control-plane's own delegated root scope (already created by
`create_and_enrol_control_plane_slice()` before this probe runs, per
`overdrive-control-plane`'s own cgroup-boot-ordering convention) and
asserts the content parses with an `oom_kill` key present — the same
kernel-guaranteed key the per-VM read depends on. Fault-injection
scenarios, mirroring `CgroupFs::probe`'s shape:

| # | Scenario | `CgroupAccountingProbeError` variant |
|---|---|---|
| 1 | Read fails (`ENOENT`, `EACCES`) | `Substrate { source }` |
| 2 | Content is not valid UTF-8 | `SubstrateCorrupt` |
| 3 | Content is valid UTF-8 but has no `oom_kill` line | `MissingOomKillKey` |

`SimCgroupAccounting` (`adapter-sim`) is an in-memory `BTreeMap<PathBuf,
u64>` with an injectable per-path error schedule, mirroring `SimCgroupFs` —
the seam that makes "this VM's scope was OOM-killed" a DST-controllable
scenario for Tier-1.

---

## Alternatives considered

### A1 — A stateful `Virtualizer`-shaped builder (the reference implementation's shape)

`configure` → `set_boot_source` → `attach_drive` → `start`, policed by a
`VmState` enum and `validate_state_*` functions.

**Rejected.** It re-checks at runtime what a value type gets at compile time,
and its stringly-typed escape hatch (`attach_drive("output", …)` intercepted on
a magic string, meaning "spawn a virtiofsd sidecar" on one backend and "attach a
block device" on the other) is intake precedent warning #5. It also forfeits what
D2 *does* deliver — one rendering site per lie, lint-bindable: under a
setter-accumulating builder every one of the three becomes "remember to call the
right setter", with no single site for a `dst-lint` clause to bind to.

### A2 — No `Vmm` port; `VmDriver` spawns Cloud Hypervisor directly

**Rejected, and by rule rather than by taste.** Process spawn, the vsock UDS and
the HTTP-over-unix API socket are real host I/O; without a port trait in
`overdrive-core` and a `SimVmm` binding, none of it is reachable from Tier-1 DST
(`.claude/rules/testing.md` § "Nondeterminism must be injectable"), and
Slice 03's fail-closed confinement AC explicitly requires the unavailable
condition to be *"injected at the `Vmm` port boundary"* because the whole test
envelope runs on one Lima kernel. Intake I-2 is user direction and is
independently forced.

Noting honestly what dst-lint would *not* have caught: `BANNED_APIS` covers
`tokio::net::{TcpStream,TcpListener,UdpSocket}` but **not** `Command` or
`UnixListener`, so the lint alone would not have blocked a portless driver. The
port is required by the DST rule, not by the current lint.

### A3 — `VmConfig` carries `image_type`, `landlock_rules`, `memory_max` and `rlimit_fsize` as fields

**Rejected.** This is the shape the slices actually specify today, and it is
exactly how C-2, C-4 and C-6 arose: Slice 03's US-VM-7 names the three paths CH
*auto-derives* and omits the one that needs a rule; no slice mentions
`image_type` at all; and no slice states the `RLIMIT_FSIZE` sizing rule. A field
is a thing a crafter can populate wrongly and a reviewer can fail to notice; a
derivation from a mandatory field is not.

### A4 — Rendering `--disk` (and the rest of CH's argv) in the adapter rather than on the value

**Rejected for `--disk` specifically; accepted everywhere else.** Argv
construction is ordinarily the adapter's job and stays there. `DiskAttachment::
to_disk_arg` is the exception because the ACL boundary is exactly where the lie
lives, and putting it on the value gives one pure, unit-testable, mutation-
targetable site in `core` instead of a convention distributed over adapter call
sites. The cost — a CH-flavoured string in `core` — is accepted, and is
consistent with intake I-2's ruling that a two-implementor trait built with one
implementor is speculative generality.

### A5 — Graceful shutdown over CH's API socket (`vm.power-button`)

**Rejected** — see D4. No `acpid` in a ~200-line PID 1, and aarch64 uses PSCI
rather than an ACPI button. Retained as the mechanism to revisit if a full guest
agent (GH #100) lands with an ACPI consumer.

### A6 — A `RootfsStaging` port for the clone, separate from `Vmm`

**Rejected.** A second port trait for a single operation, with a second sim
adapter to keep equivalent. Folding staging into `Vmm::create` keeps the port
DST-complete (one `SimVmm` stands in for the whole start path) at the cost of an
asymmetry — the adapter creates the clone and does **not** remove it — which is
principled rather than convenient: **the boot reap must be able to sweep clones
whose adapter was never constructed** (a node where `cloud-hypervisor` was
uninstalled between boots), so removal cannot live behind the port. The contract
states this in as many words.

---

## Consequences

### Positive

- Three silent substrate lies (`image_type`, the vsock Landlock grant, `memory.max`
  == guest RAM) become **structurally impossible** rather than documented, and
  two more (`RLIMIT_FSIZE` sizing, the misleading kernel rejection) become
  derived or validated ahead of the hypervisor.
- The whole VM start path is reachable from Tier-1 DST through one `SimVmm`.
- Slice 03's *"open DESIGN input: which uid/gid"* is **answered** by P5, with no
  appliance-image change and no blocker returned.
- `MemoryPlan::derive` gives GH #92's right-sizing reconciler a single
  unambiguous desired-size target, which was Slice 05's stated learning
  hypothesis.
- **Deferral D-3's reduced form is closed for the VM mid-run path** (§ D8): a
  cgroup OOM from a wrong `reserve_bytes` is now diagnosable as
  `VmOutOfMemory`, not an undifferentiated `signal: 9`. This also gives
  DELIVER's `reserve_bytes` measurement pass (§ D2.3) an honest oracle — see
  Slice 05's coupling note.

### Negative, and stated

- **`reserve_bytes` ships as a RED scaffold.** The reserve has two partial
  floors and no measured boot-path value, and RSS structurally cannot supply one
  (page tables are charged to the scope and invisible to RSS). Until DELIVER
  measures it against a real boot via `memory.current` / `memory.stat`, VM
  memory limits are not deliverable. This is a **hard DELIVER dependency**, not
  a nicety.
- **`DiskAttachment::to_disk_arg` puts a Cloud-Hypervisor-shaped string in
  `overdrive-core`.** A second `Vmm` implementor would have to move it. That is
  the accepted cost of intake I-2's one-implementor ruling.
- **The `create`/remove asymmetry is real.** The adapter stages the clone and
  never removes it; the driver and the boot reap do. The contract states it, and
  the equivalence test asserts it, but it remains a shape a reader must be told
  about.
- **`VM_BOOT_DEADLINE = 30 s` is a residual, not a fix.** SD-3's worst case —
  `pending_vm_starts × deadline` of full convergence stall for VMs that boot but
  never beacon — stands. Five such VMs freeze convergence for ~150 s. The
  structural fix is deferral **D-1** (bounding the serial dispatch path), which
  is control-plane-wide and out of this feature's scope.
- **A new `binary`-class crate (`overdrive-init`) and two musl targets** enter
  the build. Under BYO-artifact the operator must bake it into their rootfs;
  the platform ships the artifact and the contract, not the image.
- **A seventh port (`CgroupAccounting`) plus its two adapters and probe** (§
  D8) — for one read method. The cost is real: a new trait, `RealCgroupAccounting`
  / `SimCgroupAccounting`, composition-root wiring, and a fault-injection
  probe, all to close one diagnosis gap. Accepted because ADR-0083 § A8
  already rejected widening `CgroupFs` for a related need on the trait's own
  contract, and reusing that verdict here (rather than re-litigating it for a
  "just one more method" need) is what keeps the contract meaning one thing.
- **`overdrive-control-plane`'s `worker::exit_observer::handle_exit_event`
  is touched** — one additive precedence branch, `DriverType`-agnostic. Slice
  01's Dependencies list previously named this file "reused unchanged"; that
  claim no longer holds and is corrected in the slice rather than left to
  contradict the code.

### Neutral

- `--api-socket` is configured and unused by every path in this feature. It is
  present for GH #92, and its Landlock rule is auto-derived by CH.
- The `Vmm` port deliberately carries **no guest-facing surface at all** — no
  readiness, no shutdown request, no exit report. Everything guest-shaped rides
  the beacon session, which `VmDriver` owns. A reader looking for "how does the
  platform talk to the guest" will not find it on the hypervisor port, and that
  is the intended boundary rather than an omission.
- **Three review-iteration-1 corrections changed pinned surface** and are called
  out because a crafter reading only the first draft would build the wrong
  thing: `shutdown` → `terminate` (re-scoped to the process half),
  `VmExitWatch::recv(self)` → `recv(&mut self)`, and `SeccompMode` collapsed
  into `VmConfinement::seccomp_arg()`.
