# ADR-0082 — The `Vmm` port trait and the `VmConfig` value: the anti-corruption layer against Cloud Hypervisor, with the substrate lies structurally discouraged and lint-enforced

## Status

Accepted. 2026-08-11.
Decision-makers: Morgan (nw-solution-architect, DESIGN wave, third of three).
Mode: propose.
Tags: phase-2, vm-driver, ports-and-adapters, earned-trust, type-driven-design,
application-arch, GH-42.

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
    pub alloc:       AllocationId,
    pub kernel:      KernelImage,       // validated magic (lie 3 / C-7)
    pub rootfs:      RootfsPlan,        // master + master_bytes + clone destination
    pub cmdline:     KernelCmdline,     // platform-derived; NOT operator surface
    pub memory:      MemoryPlan,        // guest bytes AND cgroup max (SD-4 / C-3)
    pub vcpus:       NonZeroU8,         // derived from cpu_milli, floor 1
    pub run_dir:     VmRunDir,          // owns every path inside it (SD-2 / C-4)
    pub confinement: VmConfinement,     // identity, seccomp, nofile
    pub netns:       Option<NetnsName>, // from AllocationSpec.netns
    pub cgroup_scope: CgroupPath,
}
```

There is no `image_type` field, no `landlock_rules` field, no `memory_max`
field and no `rlimit_fsize` field. Each is a **method**:

```rust
impl VmConfig {
    /// The ONLY Landlock grant CH needs beyond its auto-derived set: the
    /// run directory, read-write (lie 4 / C-4). Derived from `run_dir` —
    /// there is no field to forget, and no operator input reaches it.
    pub fn landlock_rules(&self) -> Vec<LandlockRule>;

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
"never struct-literal-constructed" lint clause is needed at all) and to
`LandlockRule`.

#### D2.2 — the vsock Landlock gap (lie 4 / C-4) and directory exclusivity (SD-2): `VmRunDir` owns every path inside itself

```rust
/// The per-allocation run directory (SD-2 — tmpfs, one per allocation,
/// holding NOTHING else). This type owns every path inside it, which is why
/// SD-2's exclusivity is a structural property rather than a convention and
/// why C-4's Landlock grant is *derived* rather than *declared*.
pub struct VmRunDir(PathBuf);

impl VmRunDir {
    pub fn for_alloc(root: &Path, alloc: &AllocationId) -> Self;
    pub fn path(&self) -> &Path;
    pub fn vsock_socket(&self) -> PathBuf;              // <dir>/vsock       — CH binds
    pub fn beacon_socket(&self, port: VsockPort) -> PathBuf; // <dir>/vsock_<port> — the DRIVER binds
    pub fn api_socket(&self) -> PathBuf;               // <dir>/api
    pub fn console_log(&self) -> PathBuf;              // <dir>/console.log
    pub fn landlock_grant(&self) -> LandlockRule;      // rw on the DIRECTORY, never the socket
}
```

Three measured constraints are discharged by this one type:

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
    pub fn derive(declared: u64) -> Self;
    pub const fn guest_bytes(&self) -> u64;
    pub const fn cgroup_max_bytes(&self) -> u64;
}

/// The reserve policy. A **function evaluated at start time**, never a
/// persisted field (§ "Persist inputs, not derived state"): a persisted
/// reserve would be a stale cache of this policy, and — per Hera's DD-5 —
/// persisting it would *manufacture* the stored-pair invariant that would
/// then have justified a `VmInstance` aggregate.
pub fn reserve_bytes(guest_bytes: u64) -> u64;
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

**The reserve's constant is measured in DELIVER, not guessed here.** Titan
established that RSS structurally cannot supply it — host page tables for the
guest mapping are charged to the scope via `memory.stat pagetables` and are
invisible to RSS. The two honest floors are ~5.4 MiB steady-state above a 2 GiB
guest and ~11.9 MiB before guest RAM is resident. `reserve_bytes`'s body is
therefore **a `todo!("RED scaffold: measured in DELIVER via memory.current /
memory.stat, per SD-4")` until measured**, and shipping a guessed constant
between those floors is the intake-precedent-#7 "magic version floor" failure.

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

`KernelFormatError` carries the **format** problem and the arch — and
`classify_driver_failure`'s VM arm maps it to
`TransitionReason::VmKernelFormatUnsupported { path, arch, detail }`
(ADR-0083 § D5). The operator never reads `UefiTooBig`; CH's verbatim text
lives in `AllocStatusRow.detail`, never in the variant's meaning.

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
pub struct VmProcess { pub control: VmControl, pub exit: VmExitWatch }
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
let VmProcess { control, mut exit } = vmm.create(&config).await?;
let outcome = tokio::select! {
    biased;
    ready = beacon.accept_ready()          => /* guest signalled READY  → Ok(handle) */,
    ended = exit.recv()                    => /* VMM died with no beacon → Err(StartRejected) */,
    ()    = clock.sleep(VM_BOOT_DEADLINE)  => /* deadline                → Err(StartRejected) */,
};
// On the Ok path `exit` is STILL LIVE and is moved, together with the
// accepted beacon session, into the per-alloc exit watcher.
```

- **`biased;` is load-bearing.** If the beacon and the VMM exit are both ready,
  the beacon wins: a guest that beaconed and then died is a *started* VM whose
  ending belongs to the exit watcher, not to `start`. **This is only meaningful
  because `VmExitWatch::recv` borrows rather than consumes** — the watch must
  outlive the race or the exit watcher receives nothing.
- **The VMM-exit arm carries CH's stderr into the diagnosis** — that is where
  the `[D5]` "name the real problem" text lives, and it is why the arm does
  double duty regardless of how fast CH exits. Titan flagged CH's
  failure-to-exit *latency* as **unmeasured**; DELIVER measures it, and if it
  approaches the deadline, SD-3 option C (an asynchronous readiness seam) is
  the named re-opening.
- **`VM_BOOT_DEADLINE = 30 s`**, and it is a *policy constant in the driver*,
  not a persisted field and not a magic number. Derivation: the slowest
  measured substrate is **8.7 s** (nested aarch64, module loads + nesting;
  bare metal is **~1.1 s**, 12/12 runs, 16 ms spread), plus guest fsck and the
  three `CONFIG_VSOCKETS=m` module loads — 30 s is ~3.4× the worst observation.
  There is no per-workload input to persist, so § "Persist inputs, not derived
  state" is satisfied trivially; when one appears, the deadline becomes a
  function of it.
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

**`ExitKind::CleanExit` for a VM means "the guest agent reported a clean exit",
never "the platform verified the workload succeeded"** (Hera's DD-4). No
artifact may state or imply otherwise.

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
