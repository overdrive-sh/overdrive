# ADR-0083 — `DriverRegistry` replaces the single `AppState.driver`; `AllocationSpec` carries a per-driver payload; the registry *is* the VM capability gate

## Status

Accepted. 2026-08-11. **Revised 2026-08-11 (review iteration 3)** — § D7 is
rewritten from a converge-on-boot (Bar 1) pass to a registered `Reconciler`
(`reconcilers.md` **Bar 2**) **by user ruling**; A7–A9 added; `plan_vm_reap` /
`execute_vm_reap` / `VmReapPlan` **deleted**. The application-architecture shape
is pinned in `brief.md` § *Application Architecture* → **§ 105a**.
Decision-makers: Morgan (nw-solution-architect, DESIGN wave, third of three).
Mode: propose.
Tags: phase-2, vm-driver, composition-root, action-shim, spec-parse, reconciler,
application-arch, GH-42.

**Executes the migration [ADR-0022](adr-0022-appstate-driver-registry-deferred.md)
pre-committed** — *"Phase 2+ adds the second driver class … and the registry
pattern earns its keep at that point."* This is that point. ADR-0022 § "Decision"
(a single `AppState.driver: Arc<dyn Driver>`) is **superseded**; its reasoning
for deferring is preserved and unamended.

Extends [ADR-0030](adr-0030-allocation-spec-args.md) §6 (per-driver-class spec
types, pre-sanctioned) and [ADR-0031](adr-0031-tagged-workload-driver-and-driver-input.md)
(the tagged `WorkloadDriver` / `DriverInput` shape, and the deliberate
irrefutable-destructure tripwires at `:197`).

Companion: [ADR-0082](adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md)
(the `Vmm` port and the `VmConfig` value).

Implements the composition-root half of `brief.md` § *System Architecture* →
**SD-5** (*"how the composition root expresses that gate is a solution-architect
decision"*), the reconciler half of **SD-1** (its five pin obligations), and the
binding half of § *Domain Model* → **DD-1** / **DD-1(b)** / **DD-5** (*"no
reconciler may author a terminal claim on a Platform-Reclamation row"*, the
supervision precondition, and the two-`Action` split with its payload
prohibitions). None of those sections is amended; all are consumed.

Depends on `.claude/rules/development.md` § "Type-driven design", § "Errors",
§ "Newtypes — STRICT by default", § "Reconciler I/O", § "Deletion discipline";
`.claude/rules/reconcilers.md`; CLAUDE.md § "Build vertical slices through
production entry points", § "Implement to the design — never invent API
surface", § "Deferrals require GitHub issues"; intake **I-5** (one VM driver,
`[vm]`, single cut).

---

## Context

### Three gaps, one root

`crates/overdrive-control-plane/src/lib.rs:1422-1425` (inside
`compose_production_driver`, declared at `:1401`) composes exactly one driver:

```rust
let driver: Arc<dyn Driver> = Arc::new(
    overdrive_worker::ExecDriver::new(cgroup_root, clock, fs)
        .with_probe_runner(Arc::clone(&probe_runner)),
);
```

It lands in `AppState.driver: Arc<dyn Driver>` (`lib.rs:198`). **A second driver
cannot be reached without changing that line** — and intake precedent warning #1
is precisely this failing in the wild: the reference implementation's
`create_virtualizer()` has zero callers, its roadmap step 03-01 ("wire the
factory into the runner") was never executed, and `OPENCAPSULE_VMM=cloud-hypervisor`
changes nothing. **If `lib.rs:1422-1425` still composes one `ExecDriver` when this
feature closes, the feature has failed the same way.** That is `[G1]` /
system constraint 2 and it is the feature's pass/fail bar.

Two more gaps sit under it, and neither is optional:

1. **`AllocationSpec` has no driver discriminator.** Its shape is flat —
   `command: String, args: Vec<String>` (`traits/driver.rs:139-142`) — so even a
   registry has nothing to key on. The shim's `dispatch_single` reads
   `driver.r#type()` *from the driver it already holds* (`action_shim/mod.rs:1301`),
   which is circular the moment there are two — and the *stop* path has no key
   at all (§ D2a(b)).
2. **The parser has no driver-table dispatch at all.** `WorkloadSpecInput::
   from_toml_str` (`workload_spec.rs:710`) hardcodes
   `table.contains_key("exec")` → `ParseError::MissingExec` (`:743-745`) →
   `parse_section(table, "exec")` (`:753`). There is no `DriverInput` in that
   parser — the one at `aggregate/mod.rs:906` feeds the *legacy* `JobSpecInput`
   path via `#[serde(flatten)]`. So "exactly one driver table" has no
   representation to be exactly-one *of*.

### And a fourth, from the domain model

Hera's DD-1 binds sites in **two** reconcilers, and its third trap is silent:

- `is_natural_exit` (`workload_lifecycle.rs:1124-1131`) is `terminal &&
  !is_intentionally_stopped`, so a reclamation row that is *merely* "not an
  intentional stop" satisfies it. For a **Job**-kind workload the finalise branch
  (`:622-639`) evaluates **before** the restart branch (`:673`) and **returns
  unconditionally**, and `classify_natural_exit_terminal` (`:1133-1146`) falls
  through to `TerminalCondition::Failed { exit_code: Some(0) }`. **A reclaimed
  Job-kind VM is finalised as a failed job carrying a fabricated exit code, and
  is never restarted.**
- `startup_probe_failed_action` (`service_lifecycle.rs:956-996`) gates on
  `started_at.is_some()` ∧ attempts ∧ deadline ∧ no-Pass — **it never inspects
  `fact.state`**, and the enclosing loop (`:500`) filters no state either. A
  Service alloc reclaimed after Running but before Stable is handed a fabricated
  `ServiceFailed { StartupProbeFailed }` for probes that never failed.

---

## Decision

### D1 — `DriverRegistry`: a value in `overdrive-core`, replacing `AppState.driver`

```rust
// crates/overdrive-core/src/traits/driver.rs — beside the trait it indexes

/// The set of workload drivers this node composed. Executes ADR-0022's
/// pre-committed registry migration.
///
/// **Absence of a key is a first-class answer**, not an error state: a node
/// with no `cloud-hypervisor` installed simply has no `Vm` entry, and that
/// absence *is* SD-5's capability gate.
pub struct DriverRegistry { by_type: BTreeMap<DriverType, Arc<dyn Driver>> }

impl DriverRegistry {
    pub fn new() -> Self;
    /// Keyed on `driver.r#type()` — the driver names itself; no second
    /// source of truth to drift.
    pub fn insert(&mut self, driver: Arc<dyn Driver>);
    pub fn get(&self, t: DriverType) -> Option<&Arc<dyn Driver>>;
    pub fn supports(&self, t: DriverType) -> bool;
    pub fn kinds(&self) -> impl Iterator<Item = DriverType> + '_;
}
```

`BTreeMap`, not `HashMap`: `kinds()` is iterated for the admission-rejection
message and for `health.startup` logging, so ordering is observed
(§ "Ordered-collection choice").

`AppState.driver: Arc<dyn Driver>` becomes `AppState.drivers: Arc<DriverRegistry>`.
Per § "Deletion discipline" and intake I-5's single-cut ruling, the old field is
**deleted in the same PR** — no shim, no `driver()` compatibility accessor, no
grace period.

**Why a registry rather than a two-arm `match`.** Three reasons, and the third
is the one that decides it:

1. ADR-0022 pre-committed exactly this, naming the trigger.
2. A `match` in the shim puts the driver set in one function; the registry puts
   it at the composition root, where SD-5's gate already has to live.
3. **The registry expresses the capability gate as data.** SD-5 requires that a
   node without `cloud-hypervisor` *"boots normally; `[vm]` deploys are rejected
   at admission naming the absent capability"*, while a node with CH present and
   a lying substrate *refuses to boot*. With a registry, the first case is
   `!drivers.supports(DriverType::Vm)` — a missing map entry. With a `match`, it
   is a `bool` field or an `Option<Arc<dyn Driver>>` beside the match, i.e. a
   second representation of the same fact that can disagree with it.

### D2 — the composition root: discover → probe → insert, mirroring `compose_mtls`

```rust
// crates/overdrive-control-plane/src/lib.rs, in compose_production_driver

let mut drivers = DriverRegistry::new();
drivers.insert(Arc::new(ExecDriver::new(cgroup_root, clock, fs).with_probe_runner(..)));

// SD-5: the composition gate keys off an OBSERVABLE — the presence of the
// hypervisor binary — not a new operator knob. Same shape as
// `compose_mtls = config.dataplane_override.is_none()` (lib.rs:1918-1921).
match CloudHypervisorVmm::discover(&vm_layout).await {
    Ok(None) => {
        // Capability ABSENCE. Not a fault. The node boots; `[vm]` is
        // rejected at admission. No `Vm` key is inserted.
        tracing::info!(name: "driver.vm.not_composed", reason = "hypervisor_absent", ..);
    }
    Ok(Some(vmm)) => {
        // Composed ⇒ Earned Trust applies: wire → probe → use.
        if let Err(source) = vmm.probe().await {
            tracing::warn!(name: "health.startup.refused", reason = "vmm.probe", error = %source, ..);
            return Err(ControlPlaneError::VmmBoot(VmmBootError::Probe { source }));
        }
        drivers.insert(Arc::new(VmDriver::new(Arc::new(vmm), clock, fs, vm_layout)));
    }
    Err(source) => return Err(ControlPlaneError::VmmBoot(VmmBootError::Discovery { source })),
}
```

This is **composition-gated hard refusal**, and it is not a novel disposition —
`MtlsEnforcement::probe` (`lib.rs:1988-1997`) and `MtlsResolve::probe`
(`lib.rs:2011-2031`) already sit inside `if compose_mtls` (`lib.rs:1934-1935`)
and already `return Err(...)` on failure. SD-5 applies that shipping pattern to a
second optional subsystem.

**Typed all the way**, per § "Errors" → *"never flatten a typed error to
`Internal(String)` at a composition boundary"*: a dedicated
`ControlPlaneError::VmmBoot(#[from] VmmBootError)` variant, alongside the
existing `ViewStoreBoot` / `Tls` / `Cgroup` / `MtlsBoot` / `NetnsRecovery`
precedents. Never `.map_err(|e| ControlPlaneError::internal(...))`.

**The gate's inverse hazard, restated because it is a real operational
surprise** (Titan named it and it belongs in the code's own record): under this
rule, *installing* the `cloud-hypervisor` package can flip a node from booting to
**refusing to boot** — if that node's staging filesystem cannot reflink, the
probe that was previously not composed now runs and refuses. It lands at the next
`serve` boot, not at install time. That is correct behaviour, and it must be
stated or it reads as an unexplained boot failure after an unrelated package
update.

### D2a — the registry has **four** consumers, not one; three were missed in the first draft

> **Added at review iteration 1, and it is this ADR's most important
> correction.** The first draft specified the registry for the
> `StartAllocation` path only. `AppState.driver` is consumed at **four** seams,
> and replacing it with a map without pinning the other three would have
> shipped a VM that starts, cannot be stopped, whose exit is never observed,
> and which gets host-socket mTLS interception installed on a datapath its
> guest traffic never traverses. Each is pinned below.

#### (a) `exit_observer` — one observer per registry entry

`worker::exit_observer::spawn_with_runtime(obs, driver: Arc<dyn Driver>, …)`
(`exit_observer.rs:156-163`) is called **once**, at `lib.rs:2293`, with the
single `state.driver`. Inside: `driver.take_exit_receiver()` (`:165`) yields
*the one* receiver and returns early on `None`; `driver_kind` is captured once
outside the loop (`:171`) and stamped on every row as
`TransitionSource::Driver(driver_kind)` (`:209`, and `:490` on the escalation
path);
`release_for_exit_emission` routes to that one driver (`:351`). `ExitEvent`
(`traits/driver.rs:299-319`) carries **no** driver discriminator, so merging two
channels cannot recover provenance.

**Decision: one observer task per registry entry.** `lib.rs:2291-2298` becomes a
loop over the registry, each iteration spawning `spawn_with_runtime` against one
driver and capturing its own `driver_kind`. `spawn_with_runtime`'s signature is
unchanged; `release_for_exit_emission` resolves per-task, which is correct
because each task only ever sees exits from its own driver.

**And the loop reaches a FIFTH seam the four-consumer table does not name.**
*(Found at review iteration 2 — this is the same "stopped one seam short" shape
the four-consumer finding itself corrected, one layer down.)* `ServerHandle`
holds a **scalar** `exit_observer_task: tokio::task::JoinHandle<()>`
(`lib.rs:1020`) and `shutdown()` awaits exactly one (`lib.rs:1135-1136`). Two
concrete failures follow from a naive loop:

- **Dropping a tokio `JoinHandle` DETACHES; it does not abort.** N−1 observer
  tasks would survive `shutdown()` holding `Arc<dyn ObservationStore>` /
  `Arc<broadcast::Sender>` / `Arc<ReconcilerRuntime>` clones — so teardown never
  completes and observation writes can land *after* `shutdown()` returns.
- **The token is minted per call** (`CancellationToken::new()` at `lib.rs:2290`).
  A loop that mints one per driver retains only the last, and the other N−1 park
  on `rx.recv()` with **no cancel path at all** — the exact deadlock the token
  was introduced to prevent.

**Pinned:** `ServerHandle.exit_observer_task` becomes
`exit_observer_tasks: Vec<tokio::task::JoinHandle<()>>` (`lib.rs:1020`);
`shutdown()` step 4 becomes cancel-once, then
`for h in self.exit_observer_tasks { let _ = h.await; }` (`lib.rs:1135-1136`);
and the loop **clones the single `exit_observer_shutdown` token**
(`CancellationToken::clone` shares one cancellation source) rather than minting
one per driver. Per-driver tokens is the shape that leaks.

**Rejected: merge the receivers and add `driver: DriverType` to `ExitEvent`.**
It changes a core value type on every driver's emit path to solve a composition
problem, and the existing early-return-on-`None` contract (*"the test harness
wires exactly one observer per driver instance"*, `:98-99`) already says the
per-driver shape is the intended one.

**Without this, `[D3]` — Slice 01's north star — is dead on the production
path**: VM `ExitEvent`s would never reach the ObservationStore, so the guest's
exit code would never reach `workload describe`.

#### (b) the stop / terminal path — an alloc→driver index, because those Actions carry no spec

`Action::StopAllocation { alloc_id, terminal }` and
`Action::FinalizeFailed { alloc_id, terminal }`
(`reconcilers/mod.rs:411-416`, `:448-453`) carry **no `spec` and no
`workload_id`**, and `AllocStatusRow.kind` is `WorkloadKind`, which § D7 itself
pins as *not* the driver. So on the stop path the shim has no key at all.
The arms that need one: `driver.stop(&handle)` (`action_shim/mod.rs:1697`,
`:1472`), `driver.on_alloc_terminal` (`:1211`, `:1757`) and
`driver.on_alloc_stable` (`:1209`).

**Decision: `AppState` carries
`alloc_drivers: Arc<parking_lot::Mutex<BTreeMap<AllocationId, DriverType>>>`**,
written on the `StartAllocation` / `RestartAllocation` arms (where the payload
*is* in hand) and read on every stop/terminal arm.

**Lock discipline is pinned, not left to a crafter**, because the read sites
immediately `.await` a driver call (`action_shim/mod.rs:1472`, `:1697`) and that
is the textbook *"never hold a lock across `.await`"* trap
(`development.md` § "Concurrency & async") — and because `dispatch` already
carries **both** `tokio::sync::Mutex` and `parking_lot::Mutex` parameters, so
there is no default to fall back on. The shape: **lock → clone the `DriverType`
→ drop the guard → then `.await` the driver call.** `parking_lot`, per the rule.

**Lifetime, stated totally rather than as "removed on the terminal arm":** the
entry is inserted on `StartAllocation` / `RestartAllocation` and removed on the
terminal arms (`FinalizeFailed`, and `StopAllocation`'s terminal write). An
allocation that reaches neither leaves one `(AllocationId, DriverType)` entry —
16 bytes — for the life of the process. That is **bounded by allocations
started this boot**, and `RestartAllocation` reuses the same `alloc_id`
(`workload_lifecycle.rs:744`), so a restart re-inserts the same key and the map
does not grow per restart. Read-then-write on the restart arm: the stop-half
read at `:1472` happens **before** the re-insert.

**Both signatures are pinned, and both must change** — the first fix pinned only
`dispatch` and mis-cited it. `action_shim::dispatch` is declared at
`action_shim/mod.rs:671` (`:852` is an *argument*, `state.driver.as_ref()`,
inside `dispatch_with_workflow_intent`'s call at `:842`). But the index is read
and written inside **`dispatch_single`** (`:983`), which `dispatch` calls — so
pinning only `dispatch` leaves a crafter inventing a parameter on
`dispatch_single`, on the one function this ADR calls the `[G1]` pass/fail bar,
for exactly the reason it gives for pinning it. CLAUDE.md
§ *"never invent API surface"* forbids that.

```rust
// action_shim/mod.rs:671 — dispatch
pub async fn dispatch(
    actions: Vec<Action>,
    drivers: &DriverRegistry,          // was: driver: &dyn Driver
    alloc_drivers: &AllocDriverIndex,  // NEW — the routing index
    /* …remaining parameters unchanged… */
) -> Result<(), ShimError>;

// action_shim/mod.rs:983 — dispatch_single, same two parameters
async fn dispatch_single(
    action: Action,
    drivers: &DriverRegistry,
    alloc_drivers: &AllocDriverIndex,
    /* …remaining parameters unchanged… */
) -> Result<(), ShimError>;

pub type AllocDriverIndex = parking_lot::Mutex<BTreeMap<AllocationId, DriverType>>;
```

`dispatch_with_workflow_intent` (`:842`) is the call site that supplies
`state.driver.as_ref()` today and changes with them.

A miss resolves to a typed `ShimError::UnknownDriverForAlloc { alloc_id }` and
is **never** silently routed to `ExecDriver` — calling `ExecDriver::stop` on a
VM alloc returns `DriverError::NotFound`, which `let _ =`
(`action_shim/mod.rs:1694-1697`) swallows, and the result is exactly the
GiB-scale unstoppable orphan SD-1 exists to prevent.

**Rejected: add `workload_id` to `StopAllocation` / `FinalizeFailed` and do
D7's intent-side join.** It is the more principled shape and it widens two
`Action` variants consumed by three reconcilers — a change this feature's scope
does not justify when an in-memory index reconstructed from the same
`StartAllocation` the shim already handles is sufficient. **Named as the
follow-on** if a third driver arrives. Note the index is *in-memory and
per-boot*, which is correct: after a `serve` restart there are no live VM allocs
to stop — the boot-epoch drive has already reclaimed them (SD-1), and any that a
failed stop later strands are reclaimed by the steady-state sweep rather than
needing an index entry.

#### (c) `MtlsInterceptWorker` — gated to `Exec`, and the reason is #222's whole premise

`MtlsInterceptWorker::start_alloc` is fired for **every** allocation reaching
`Running` on an mTLS-composed boot — the two call sites gate on
`state == AllocState::Running` (`:1400`, `:1632`) and `mtls_worker.is_some()`
(`:1424`, `:1642`), and on **nothing driver-shaped**
(`action_shim/mod.rs:1425`, `:1643`). Its own
docstring states the predicate is `DriverType::Exec`, *"which is
unconditionally true on the worker's exec lifecycle path"*
(`mtls_intercept_worker.rs:474-477`). **A `VmDriver` makes that false**, and
the install is fail-closed (`:482-497`) — so on an mTLS-composed boot a VM
allocation either gets host-socket egress interception installed on a veth its
guest traffic never traverses (a silent false confidentiality claim), or fails
to install and the VM is killed.

**Decision: gate both call sites on
`spec.driver.driver_type() == DriverType::Exec`.** A microVM terminates TCP
**inside the guest**, so `cgroup_connect4` / sockops are structurally blind to
it — that is GH [#222](https://github.com/overdrive-sh/overdrive/issues/222)'s
entire premise, already cited by this feature for the `[vm]` + `[service]`
rejection (§ D4). Until #222 lands, a VM allocation is honestly *not*
mesh-enrolled, which is also why Slice 02 refuses `[vm]` + `[service]` at
deploy time rather than shipping a VM that looks enrolled and is not.

`provision_and_inject_netns` (`:906`, called `:1281`, `:1496`) is **not**
gated: a VM allocation still gets its netns slot. An empty netns is *stronger*
confinement, not a gap (Slice 01 already argues this), and ADR-0082 § D6 makes
`config.netns == None` a supported case for the mTLS-uncomposed boot rather than
a VM-specific one.

### D3 — `AllocationSpec` carries a tagged per-driver payload

```rust
// crates/overdrive-core/src/traits/driver.rs

pub enum DriverPayload { Exec(ExecPayload), Vm(VmPayload) }

pub struct ExecPayload { pub command: String, pub args: Vec<String> }

pub struct VmPayload {
    pub command: String,          // the command run INSIDE the guest
    pub args:    Vec<String>,
    pub kernel:  PathBuf,         // operator surface, BYO artifact
    pub rootfs:  PathBuf,         // operator surface, BYO artifact
    pub volumes: Vec<VmVolume>,   // Slice 04; empty for Slices 01–03
}

impl DriverPayload {
    pub fn driver_type(&self) -> DriverType;   // the routing key
    pub fn command(&self) -> &str;             // both variants have one
    pub fn args(&self) -> &[String];
}
```

`AllocationSpec.command: String` and `AllocationSpec.args: Vec<String>` are
**replaced** by `AllocationSpec.driver: DriverPayload`. Every existing read
becomes `spec.driver.command()` / `spec.driver.args()`; the change is
compiler-enforced at every site, which is exactly ADR-0031's deliberate
tripwire discipline.

**Rejected: `AllocationSpec.vm: Option<VmPayload>` alongside the existing flat
fields.** Cheaper, and it manufactures an invalid state — `vm: Some(..)` with a
meaningful top-level `command` — which is the sentinel anti-pattern
§ "Type-driven design" forbids by name.

`AllocationSpec` derives neither serde nor rkyv (`traits/driver.rs:132`), so
this change is **not** a schema-evolution event and triggers no envelope bump.

### D4 — the parse surface: "exactly one driver table", replacing `MissingExec`

`WorkloadSpecInput::from_toml_str` gains a driver-table dispatch. `[exec]` and
`[vm]` are siblings; exactly one is required. `ParseError::MissingExec`
(`workload_spec.rs:75`) is **deleted** and replaced by two variants:

```rust
#[error("missing required section: exactly one of [exec] or [vm] is required")]
MissingDriverSection,
#[error("both [exec] and [vm] are present; exactly one driver section is required")]
MultipleDriverSections,
```

This mirrors the existing `MixedServiceAndJob` / `MissingKindSection` pair
(`:55`, `:71`) — the same shape, one axis over. ADR-0031's *table-name-is-the-
discriminator* property holds: the table name equals the `DriverType` tag
(`[vm]` ↔ `DriverType::Vm`).

**`[vm]` + `[service]` is rejected at deploy time** (Slice 02 / US-VM-6) — no
intent committed, no allocation created — with a message naming guest
networking, guest-reachable probes and guest-stack mTLS interception and citing
GH [#257](https://github.com/overdrive-sh/overdrive/issues/257) and
[#222](https://github.com/overdrive-sh/overdrive/issues/222). `[vm]` + `[job]`
and `[vm]` + `[schedule]` are accepted. This is a *semantic* rejection with
guidance, mirroring `ParseError::ProbesNotAllowedOnKind` (`:827`).

**The capability rejection is separate from the parse rejection**, and the split
matters: a `[vm]` spec on a node with no `Vm` driver composed is **syntactically
valid** and fails at *admission*, naming the absent capability and the node.
Putting it in the parser would make a host property look like a spec property.
This also **refines Slice 02**, which treats *"no `cloud-hypervisor` on the
host"* as a per-deploy failure: it is a host property that cannot change between
deploys on the same node, so SD-5's boot probe (and, for the absence case, the
registry) is a strictly better source. Slice 02's ACs are unaffected — the deploy
still fails; the message improves.

### D5 — `classify_driver_failure`'s `DriverType` parameter stops being unused

`classify_driver_failure(text, _driver: DriverType, _command: &str)`
(`action_shim/mod.rs:179-202`) documents both parameters as *"accepted for
forward-compatibility … Phase 1's prefix table is `ExecDriver`-shaped only"*.
That forward compatibility is now cashed: the function branches on `driver`
first, and the exec prefix table is reached only under `DriverType::Exec`.
**Zero exec test cases change** — Slice 02's stated learning hypothesis, and its
falsifiable form.

The VM arm maps `DriverError::StartRejected.reason` — which `VmDriver` builds
from a **typed** `VmmError`, not from free text — onto these
`TransitionReason` variants. **Cause-variant naming was re-assigned to me by
Hera's DD-3** (she delivered the Disposition name and C-7's meaning, and handed
the Cause variants over rather than dropping them):

| # | Variant | Payload | Slice | Notes |
|---|---|---|---|---|
| 1 | `VmKernelNotFound` | `{ path: String }` | 02 | |
| 2 | `VmRootfsNotFound` | `{ path: String }` | 02 | |
| 3 | `VmHypervisorAbsent` | `{ searched: Vec<String> }` | 02 | Names the paths searched — D2's refinement |
| 4 | `VmBootDeadlineExceeded` | `{ deadline_ms: u64, console_tail: Option<String> }` | 02 | The guest never beaconed |
| 5 | `VmKernelFormatUnsupported` | `{ path: String, arch: String, detail: String }` | 02 | **C-7.** Says *format*, never "size cap". CH's verbatim `UefiTooBig` text goes in `AllocStatusRow.detail` |
| 6 | `VmConfinementUnavailable` | `{ control: ConfinementControl, detail: String }` | 03 | The **fifth** variant US-VM-7 asks for — one variant, typed discriminant |
| 7 | `VmGuestExitUnreported` | `{ vmm_exit_code: Option<i32>, vmm_signal: Option<u8> }` | 03 | The hypervisor ended with no agent report |
| 8 | `VmVolumeSourceNotFound` | `{ path: String }` | 04 | |
| 9 | `VmStorageDaemonAbsent` | `{ searched: Vec<String> }` | 04 | |
| 10 | `VmGuestMountFailed` | `{ target: String, detail: String }` | 04 | The composite-lie case |
| 11 | `VmStorageSocketTimeout` | `{ socket: String, waited_ms: u64 }` | 04 | |
| 12 | `VmStorageSandboxUnavailable` | `{ requested: String, detail: String }` | 04 | |

```rust
pub enum ConfinementControl { Landlock, Seccomp, UidDrop, RlimitFsize, RlimitNofile, KvmAccess }
```

**Why #6 is one variant and not six**, against US-VM-2's *"no two distinct
causes share a variant"*: the distinct causes there are all *"this host cannot
supply confinement control X"* — one cause class discriminated by a typed field,
which is what Slice 03 asks for in as many words (*"a **fifth** variant minted
in Slice 02's shape"*). A `String` discriminant would have been the stringly-
typed version and is rejected.

**Twelve distinct causes**, against K3's "≥ 4 distinct". Per DD-3, the
reclamation **disposition** is deliberately **not** in this table and must not
be counted toward K3 — counting a disposition as a failure cause would let the
feature satisfy K3 without shipping a fourth diagnosis.

`TransitionReason` is `#[non_exhaustive]` (`transition_reason.rs:87`) and every
addition is appended, preserving rkyv discriminants — the same discipline
`StoppedBy` states verbatim at `:237-241`.

**Two documentation corrections land in the same commits**, because this
vocabulary is the one being touched and § "Documentation" forbids leaving a
false claim standing:

- The emit-inventory row at `transition_reason.rs:55` marks `NoCapacity`
  emitted **`yes`** while it has **no production construction site** in the
  workspace (the live `PlacementError::NoCapacity` at `scheduler.rs:70` is a
  *different type*). Correct it to `NO`. This is Hera's **H-4(a)** — a false
  documentation claim, not a deferral. Contrast `OutOfMemory` at `:56`, which
  correctly declares itself `NO — Phase 2`.
- The emit-inventory table is missing rows for `MtlsInterceptInstallFailed`
  (`:193`) and `WorkloadNetnsProvisionFailed` (`:209`) — 15 rows against 17
  variants. Add them while adding the twelve.

### D6 — DD-1, bound: one predicate, two reconciler sites, and a totality property

`StoppedBy::PlatformReclaimed` is appended (discriminant 4), per Hera's DD-1.
The reclaimed row is `state: Terminated`, `reason: Some(Stopped { by:
PlatformReclaimed })`, `terminal: None` — `Failed` is excluded on domain grounds
*and* because it would open `service_lifecycle.rs:611`'s EarlyExit fabrication.
**No parallel boolean flag anywhere**; the class is on the row.

One new public predicate, co-located with the vocabulary it reads
(§ "Label enums own their string representation"):

```rust
// crates/overdrive-core/src/transition_reason.rs

/// True iff this terminal row is a Platform Reclamation (DD-1): the platform
/// destroyed one runtime instance while the workload's intent still stands.
/// Reads `reason` OR `terminal`, mirroring `is_intentionally_stopped`'s shape.
#[must_use]
pub fn is_platform_reclaimed(row: &AllocStatusRow) -> bool;
```

**Site 1 — `workload_lifecycle.rs`, three edits** *(the third was missed in the
first draft and found at review iteration 1)***:**

```rust
fn is_natural_exit(row: &AllocStatusRow) -> bool {
    row.state.is_terminal() && !is_intentionally_stopped(row) && !is_platform_reclaimed(row)
}
```

…which is the **only** predicate that changes meaning, and it is what stops the
Job finalise branch (`:622-639`) firing so the row falls through to the restart
branch (`:673`). `is_intentionally_stopped` (`:1096-1111`) and `is_restartable`
(`:1116-1120`) are **unchanged**: `PlatformReclaimed` fails the former's
`Operator | SystemGc` match for free, and therefore satisfies the latter for
free.

And the restart branch, on a reclaimed row, **writes no View field at all**:

```rust
// :786-799, guarded
if !is_platform_reclaimed(failed) {
    *next_view.restart_counts.entry(..).or_insert(0) += 1;      // :788-789
    next_view.last_failure_seen_at.insert(..., tick.now_unix);  // :799
}
```

`last_failure_seen_at` is exempt for the same reason `restart_counts` is:
it is **failure memory**, and a reclamation is not a failure. Stamping it would
also make the reclaimed workload serve a backoff window before coming back —
the opposite of SD-1's *"lets the existing restart/backoff reconciler re-drive
them."* This **extends Hera's DD-5 declared universe** by one slot, and declares
it complement-equal; it does not contradict it. *(Residual, stated: an alloc
with a **prior genuine failure** still carries a stale `last_failure_seen_at`
entry, which the backoff read at `:720` will consult when the reclaimed row's
restart is evaluated. The effect is at most one already-elapsed backoff window
on a workload that had already failed; it is not worth a pruning mechanism, but
it is not zero and is recorded rather than glossed.)*

**And the third edit — the backoff-ceiling branch, which authors a terminal
claim on a reclaimed row.** The first draft claimed DD-1 was "bound at three
lines" and enumerated `ServiceLifecycle`'s five action-emitting sites
exhaustively while **not** doing the same for `WorkloadLifecycle`. It has a
second `FinalizeFailed` emitter:

```rust
// :679-708, inside the restart branch the reclaimed row now reaches
if attempts >= RESTART_BACKOFF_CEILING { … Action::FinalizeFailed {
    terminal: Some(TerminalCondition::BackoffExhausted { attempts }) } }   // :703
```

Trace: guarding `is_natural_exit` routes a reclaimed row to `:673`;
`is_restartable` is true (satisfied "for free"); `attempts` is read from
`view.restart_counts` (`:678`), and because `RestartAllocation` reuses the same
`alloc_id` (`:744`) those counts **accumulate across the workload's genuine
prior failures**. A workload that had already failed five times and is then
reclaimed by a `serve` restart hits `attempts >= 5`; the idempotency guard at
`:687` reads `failed.terminal`, which a reclamation row carries as `None`, so it
does **not** short-circuit — and `BackoffExhausted` is fabricated on a row whose
workload never failed. **That is DD-1's rule violated on the very branch the
first two edits route the row into**, and it falsifies this design's own
availability claim that reclamation causes no `RestartBudgetExhausted` cascade.

```rust
if !is_platform_reclaimed(failed) && attempts >= RESTART_BACKOFF_CEILING { … }
```

The ceiling is skipped for a reclaimed row, which is consistent with the budget
exemption rather than an additional exception to it: a reclamation neither
consumes budget nor is judged against it.

**And the exhaustive `WorkloadLifecycle` audit the correction demanded, done.**
*(Added at review iteration 2 — the first fix diagnosed the asymmetry, added one
site, and still did not enumerate, which left the audit gap live.)* Five
production sites emit `terminal: Some(..)`:

| Site | Action | Verdict |
|---|---|---|
| `:390-393` | `StopAllocation { Stopped{Operator} }` | **no change** — filters `r.state == AllocState::Running`; a reclaimed row is `Terminated` |
| `:439-442` | `StopAllocation { Stopped{SystemGc} }` | **no change** — same `Running` filter |
| `:515-518` | `StopAllocation { Stopped{Operator} }` (ADR-0073 R2 running-origin replacement) | **no change** — same `Running` filter |
| `:635-638` | `FinalizeFailed` (Job natural exit) | **guarded** via `is_natural_exit` |
| `:703-706` | `FinalizeFailed { BackoffExhausted }` | **guarded** via the ceiling clause above |

The three unchanged sites are in **P2's scope** (it forbids
`StopAllocation { terminal: Some(_) }` on a reclaimed row too), so the property
covers them whether or not this table is maintained — which is the point of
preferring P2 to a site list.

**Site 2 — `service_lifecycle.rs`, one edit.** `ServiceAllocFact` gains
`platform_reclaimed: bool`, hydrated from the row via `is_platform_reclaimed`.
(This is a **hydrated `State` field, not a `View` field** — recomputed every
tick from the row, so § "Persist inputs, not derived state" is not engaged.)
`startup_probe_failed_action` returns `None` for a reclaimed fact, before its
three gates. The other four action-emitting sites are checked and need **no**
change: branch (a') `:557` and branch (a) `:580` gate on `Running`; the EarlyExit
branch `:611` gates on `Failed`; the liveness branch `:769` gates on `Running` —
a reclaimed alloc is `Terminated` and reaches none of them.

`update_startup_attempts` also needs no change, and the reason is checked rather
than assumed: it is driven by `fact.latest_startup_probe`, not by state, and a
reclaimed alloc produces no new probe result, so attempts do not move.

**Enforcement — TWO properties, and the second is the one DD-1 actually
asserts.** *(Corrected at review iteration 1: the first draft proposed only the
predicate property, which is structurally incapable of catching the
`:703` defect above — DD-1 is a statement about **emissions**, not about
predicates.)*

```
P1 (predicate totality/disjointness):
    for every AllocStatusRow with state.is_terminal(),
    exactly one of { is_intentionally_stopped, is_platform_reclaimed,
                     workload-failure (neither) } holds.

P2 (emission — the binding one), over BOTH reconcilers:
    for every reconcile whose observed allocs include a row where
    is_platform_reclaimed(row), the returned Vec<Action> contains
    NO FinalizeFailed for that alloc_id, and no StopAllocation for it
    carrying terminal: Some(_).
```

P2 is the direct transcription of *"no reconciler may author a terminal claim
on a Platform-Reclamation row"* and it holds against reconcilers that do not
exist yet, which is the property Hera's general form was written for. P1 alone
would have passed the whole first draft.

**Rejected: an `EndingClass` enum** unifying the three predicates. It would make
totality structural — genuinely better — but it is a refactor of a working
classifier with call sites across two reconcilers and the CLI renderer, for a
property the proptest above pins at a fraction of the blast radius. Hera's reuse
gate independently returned *"no new type"*. Recorded as the shape to reach for
if a fourth class ever appears.

**And two things that must NOT be done, both named because they are the
tempting moves:**

- **Do not "fix" `CrashFacts::advance` to exempt reclamation.** It already
  produces the right answer — the reap writes terminal, the restart writes
  `Running` at the same LWW key (`RestartAllocation` reuses `failed.alloc_id`,
  `:743-746`), and `advance` (`observation_store.rs:1144-1159`) snapshots into
  `last_terminated` and increments `restart_count`. Exempting it would **erase
  the occurrence**, which is ADR-0078's own defect reproduced in the feature
  that cites ADR-0078.
- **Do not zero `AllocStatusRow.restart_count`.** Budget and occurrence are two
  different quantities and the codebase already distinguishes them
  (`observation_store.rs:1210-1228`). `WorkloadLifecycleView.restart_counts` is
  the **budget** and is exempt; `AllocStatusRow.restart_count` is the
  **occurrence** and **must increment**. One English word covers both, which is
  exactly how a crafter zeroes the wrong one.

`CrashFacts::advance`'s *"unreachable in Phase 1"* docstring clause
(`observation_store.rs:1122-1132`) becomes false the moment reclamation lands —
`Terminated → Running` on the same key becomes reachable, and reachable
**correctly**. It is amended in the same commit; its advice still stands for
*operator* stops, which remain excluded upstream by `is_restartable`.

### D7 — SD-1's reclamation is a registered `Reconciler` (`reconcilers.md` **Bar 2**) whose plan is a pure function and whose plan-values are `Action`s

> **Revised 2026-08-11 (review iteration 3), by user ruling.** This section
> previously specified `overdrive-control-plane::vm_reap` as a **converge-on-boot
> (Bar 1)** pass modelled on `veth_provisioner::provision`. That verdict was
> reached by *analogy* without running the Bar-1-vs-Bar-2 test — *does `actual`
> drift while the system is up?* — whose honest answer is **yes** (SD-1's triage:
> a clone leaked by a crash between teardown steps; a scope or run directory
> stranded by a failed stop, leaving the VM **unstoppable until the next `serve`
> restart**; and SD-2's unbounded-over-lifetime clone leak swept only at the
> restart cadence). **The plan/execute split reshapes; it is not lost** —
> `reconcile` is the pure diff, the `Action`s *are* the plan, the executors are
> the impure half. `plan_vm_reap` / `execute_vm_reap` / `VmReapPlan` are
> **deleted**, not deprecated (intake I-5's single cut).

The full application-architecture shape is pinned in
`brief.md` § *Application Architecture* → **§ 105a**, which governs. What this
ADR fixes are the decisions a crafter could otherwise re-litigate:

```rust
// crates/overdrive-core/src/reconcilers/vm_reclamation.rs
// const NAME = "vm-reclamation"; TargetResource = node/<node_id>.

/// PURE — and it takes NO port, so "the observe pass wrote something" is not
/// representable rather than merely reviewed for. Mandatory mutation target.
pub fn plan_reclamation(
    desired: &VmReclamationState,
    actual:  &VmReclamationState,
) -> Vec<Action>;

impl Reconciler for VmReclamation {
    type State = VmReclamationState;
    type View  = VmReclamationView;          // FIELD-LESS, per ADR-0079
    fn reconcile(&self, d: &Self::State, a: &Self::State, _v: &Self::View, _t: &TickContext)
        -> (Vec<Action>, Self::View)
    { (plan_reclamation(d, a), VmReclamationView::default()) }
}
```

**Three things are pinned and must not be improvised** (CLAUDE.md § *"Implement
to the design"*):

1. **Two `Action` variants, never one with a flag** — `ReclaimAllocation
   { alloc_id }` authors an ending (Platform Reclamation);
   `DiscardStrandedArtifacts { alloc_id }` authors none (Artifact Disposal).
   Both payloads are `AllocationId` **and nothing else**: no disposition
   parameter (`StoppedBy::PlatformReclaimed` is constant for the first — the
   variant *is* the class) and **no regime field**, because a `boot_epoch` /
   `is_boot` flag would put the kill-authorising check on a self-declared boolean
   instead of on the observed live-handle set. Specified by **DD-5**, which is
   binding; naming and placement are this ADR's. `Action` derives neither serde
   nor rkyv (`reconcilers/mod.rs:367`), so appending after `LivenessExhausted`
   (`:615`) is **not** a schema-evolution event.
2. **The supervision discriminator is an observed input on `actual`**, never a
   `View` marker — `SupervisionSet { Unavailable (Default) | Observed(set) }`,
   read through a new **defaulted, sync** `Driver::live_allocations(&self) ->
   Option<Vec<AllocationId>>` (default `None`, which the caller reads as
   `Unavailable`). `Unavailable` authorises **nothing**: absence of evidence is
   not evidence of absence, and here the substitution would gate whether a live
   VM is killed. **No registry `Vm` entry ⇒ `Observed(∅)`**, because a node with
   no VM driver provably holds no VM supervision handle — which is what lets an
   uninstalled-`cloud-hypervisor` node still reclaim.
3. **`VmHostState::observe()` is the hydration seam** — one call returning a
   plain `VmHostObservation` (SD-1's pin 1, so #197's generalisation is a
   refactor of an existing seam rather than a rewrite). It is a **new driven
   port** rather than a widened `CgroupFs`, because `CgroupFs` is deliberately
   write-only (`traits/cgroup_fs.rs:58-257`) and two of the three surfaces are
   not cgroupfs at all; and because without a port the reconciler's `actual` is
   unreachable from Tier-1 DST.

**The wake has TWO halves, and Bar 2 supplies neither for free.**
`spawn_convergence_loop` (`lib.rs:2427-2477`) is purely broker-driven: it drains
`broker.drain_pending()` and does nothing when the broker is empty. There is no
bootstrap sweep of existing intent, and `has_work` only *re*-enqueues a
reconciler that already ticked.

**(a) After a row write** — the in-tree pattern for *"I wrote a terminal row, now
nudge the reconcilers"* lives in the exit observer, and reclamation
**deliberately bypasses the exit observer** (§ DD-2: no `ExitEvent`, no watcher;
`execute_reclaim_allocation` authors its row directly). **The exit observer
submits FOUR evaluations per exit, not three, and the executor submits all
four.** *(Corrected at review iteration 2 — the first fix miscounted the
precedent it was copying, and the omission was not cosmetic.)*

| Reconciler | Site | Why `execute_reclaim_allocation` must submit it |
|---|---|---|
| `workload_lifecycle` | `worker/exit_observer.rs:234` | The restart re-drive. SD-1's stated premise |
| `backend_discovery_bridge` | `:254` | The reclaimed alloc leaves the running set, so the backend row must lose it |
| `service_lifecycle` | `:295` | Same, on the readiness half |
| **`svid_lifecycle`** | **`:318-320`** | **Load-bearing and nearly missed.** `svid_lifecycle` converges `¬running ∧ held → DropSvid` (`svid_lifecycle.rs:316-317`, `:506-513`), and `worker/exit_observer.rs:318-320` is its **only** on-exit producer. A reclamation-authored terminal row flips `desired` to ¬running with nothing enqueuing svid-lifecycle — so `DropSvid` never fires and **the node keeps the dead allocation's leaf private key**. That is ADR-0067 O2's leak-resistance property broken on every `serve` restart that reclaims a VM |

All four sites are deliberately **unconditional** in the observer, each with an
in-code justification for why kind-gating was refused, and each documents that a
spurious enqueue costs exactly one empty reconcile
(`worker/exit_observer.rs:277-294`, `:307-317`). Submitting all four is therefore
both the safe shape and the one consistent with the precedent.
**`execute_discard_stranded_artifacts` submits none**, and has no broker and no
`ObservationStore` parameter at all — it writes no row, so there is nothing to
nudge, and the absence of both parameters is what makes DD-5's *"declared delta
empty over the observation universe"* structural instead of remembered.

The `workload_id` those four `TargetResource`s need is **not** carried in the
`Action` payload (DD-5 pins it to `alloc_id` and nothing else): the executor
re-reads the row it is about to supersede, via the existing
`find_prior_alloc_row(obs, alloc_id)` helper (`action_shim/mod.rs:1892`). That is
the same *"the executor re-observes"* rule that keeps the artifact list out of the
payload — an observation carried from the diff into the plan goes stale between
emit and execute. SD-1's *"lets the existing restart/backoff reconciler re-drive
them"* is then true by construction rather than by assumption.

**(b) The steady-state tick itself** *(added at iteration 3 — the Bar-1 draft did
not need it, and the Bar-2 ruling does not supply it)*. Because the broker is
event-driven and nothing in tree names `vm-reclamation`, **this reconciler would
never tick.** `spawn_convergence_loop` therefore submits one
`Evaluation { vm-reclamation, node/<node_id> }` every
**`VM_RECLAMATION_SWEEP_INTERVAL = 30 s`**, measured on the loop's already-injected
`Clock` so the cadence is DST-controllable rather than wall-clock. The interval
bounds the unstoppable-orphan window and the stranded-clone repair latency — the
two drifts SD-1's triage turns on — while sitting ~300× above the tick cadence so
the three-surface walk never lands on the hot path. The boot drive additionally
submits **one** evaluation on completion, so the first steady-state tick does not
wait a full interval. Three alternatives were rejected: a `last_swept_at` **View
field** (a marker on the emit path, which SD-1's pin 2 and ADR-0079 both refuse,
and which the runtime fsyncs *before* dispatching so it would record "last
attempted"); **unconditional self-re-enqueue** via `Action::EnqueueEvaluation`
(a 100 ms poll of three filesystem surfaces); and **event-only wakes** from the
allocation lifecycle (repairs only drift that had an event, quietly re-deriving
converge-on-*event* and giving back the continuous convergence the ruling
bought).

**The settle contract, because the next boot pass reads the same tree — and it is
a POSTCONDITION OF THE PORT, not a rule the boot drive must remember.**
`VmHostState::kill_scope` MUST NOT return until that scope's `rmdir` has
succeeded or returned `NotFound`. `adopt_on_restart_recovery`
(`veth_provisioner.rs:2099`) reads
`overdrive.slice/workloads.slice/<alloc>.scope/cgroup.procs` via
`alloc_scope_pids` (`veth_provisioner.rs:1988-1994`) and treats **any**
non-benign-absence io error as `NetnsRecoveryError::ObserveRead` (`:1994`),
which **refuses the boot** (`lib.rs:2146`). A scope mid-deletion, or a
`cgroup.kill` still draining, produces exactly that error class.
Reclaim-before-adopt fixes the *leak*; the settle contract is what stops the fix
causing a boot refusal. **Per SD-1 the obligation binds the boot drive only** —
a steady-state tick has no such adjacency — and putting it on the port method is
how the boot drive inherits it structurally, at zero cost to the tick. It is
asserted in `vm_host_state_equivalence.rs` across both adapters.

**Observe three surfaces**, per SD-1: every `<alloc>.scope` under
`overdrive.slice/workloads.slice/` and its `cgroup.procs`; every directory under
the VM run root; every per-launch clone in the image directory. Cross-reference
against the allocation set.

**The three surfaces are NOT equally attributable, and the diff must not treat
them as if they were** *(a precision added at iteration 3; SD-1's prose leaves it
implicit)*. The cgroup scope tree is **shared with exec allocations** —
`overdrive.slice/workloads.slice/<alloc>.scope` carries no driver — so a scope
whose allocation row is unknown is unattributable and **is left alone**, which is
exactly what preserves `ExecDriver`'s survive-a-restart behaviour. The run
directory (SD-2's exclusive per-allocation tmpfs dir) and the per-launch clone
(whose *filename* carries the allocation id, ADR-0082 § D2) **are** VM-exclusive
by construction, so an entry there with no row is an unknown **VM** allocation
and is disposed of. Consequence: **a scope is never the sole trigger** — a
reboot-orphaned VM is caught by its clone, which is the only surface that
survives a host reboot, and the executor kills the scope anyway.

***"Is this a VM allocation" is a two-surface join, not a row field.***
`AllocStatusRow.kind` is `WorkloadKind ∈ {Job, Service, Schedule}` (ADR-0047) —
it does **not** carry the driver. The pass resolves the row's `workload_id`
against the intent aggregate and matches `WorkloadDriver::Vm`. Both stores are up
before the boot passes run. Hera pinned this as language (DD-4); it is repeated
here because assuming a row field that does not exist is how this pass would
quietly become unimplementable.

**Authority rule when surfaces disagree** (SD-1, verbatim in effect): the
**cgroup scope** is authoritative for *is it alive*; the **intent-side
`WorkloadDriver`** is authoritative for *is this a VM allocation*. The run
directory is **not** — it is an *epoch* marker, absent for every VM after a host
reboot. Every disagreement converges toward *gone*, never toward *adopt*. Keying
kind on the allocation row confines the reap to VM allocations, so
`ExecDriver`'s survive-a-restart behaviour for **process** workloads is
unchanged.

**The same pass sweeps reboot-orphaned clones**, which is why
`RootfsPlan::for_alloc` derives the clone filename from the allocation id
(ADR-0082 § D2): after a host reboot the tmpfs run directory is gone and the
filename is the only remaining attribution. Slice 03's *"no leaked … rootfs
copies after terminal states"* does not cover this case — there is no allocation
left to key a terminal-state GC off.

**Ordering is load-bearing.** The **boot-epoch drive**
(`vm_reclamation_boot::converge`) runs in `run_server` **immediately before** the
`if state.mtls_worker.is_some()` block that calls
`veth_provisioner::adopt_on_restart_recovery` (`lib.rs:2131-2147`) — outside
that gate, because VM allocations exist whether or not mTLS is composed. Both
passes walk `overdrive.slice/workloads.slice/*/cgroup.procs`; adopt-first would
adopt a netns slot for an allocation the drive is about to destroy, and that netns
would then escape the same pass's orphan GC. Reclaim-first leaves an empty scope,
so the adopt pass correctly treats the netns as orphaned and reclaims it. The
pinned order becomes: **VM reclamation (boot epoch) → adopt → GC → sweep →
serve.**

**It is not a second implementation** (SD-1 pin 4). One observation
(`VmHostState::observe`), one pure diff (`plan_reclamation`), one executor pair —
the boot drive calls the two executors **directly** on the returned
`Vec<Action>`, rather than routing through `dispatch_single`, only because that
function takes fifteen parameters including `driver`, `dataplane`, `ca`,
`identity` and `mtls_worker` (`action_shim/mod.rs:983`), none of which
reclamation touches. Dragging the whole shim into the boot sequence would add
coupling, not share code; the `Action` values are the plan on both paths, which
is what makes them the same mechanism.

**Registration, and the one place this ADR sharpens SD-1's mechanism while
preserving its property.** SD-1 pin 5 asks for (a) no tick interleaving with the
boot passes and (b) registration not gated on `Vmm` composition. **(b) holds
verbatim**: `runtime.register(vm_reclamation(node_id))` is unconditional,
alongside the existing seven at `lib.rs:1525-1773`, and `VmHostState` is composed
unconditionally too — so a node that uninstalled `cloud-hypervisor` still
observes and still reclaims. **(a) holds, by a different mechanism than
"register last"**: `ReconcilerRuntime::register` takes `&mut self` and
`let runtime = Arc::new(runtime)` at **`lib.rs:1774`** precedes `AppState`'s
construction, which the boot passes at `:2131-2147` then read — so registering
after them would need `Arc::get_mut`, interior mutability, or an `AppState`
restructure. It is also unnecessary: **registration is inert** (it probes the
`ViewStore` and `bulk_load`s views, `reconciler_runtime.rs:239-329`; it drives no
tick), and the only production driver of ticks is `spawn_convergence_loop`,
spawned at **`lib.rs:2314-2320` — strictly after** the boot passes. That spawn
ordering is the load-bearing fact and is pinned as such.

**No shutdown-time stop is added.** One mechanism, not two: a graceful shutdown
path fails exactly when it matters (SIGKILL, host crash, OOM), so the boot drive
must exist regardless — and once it exists a second path buys nothing but a
second thing to keep correct.

**Reclamation is not `Vmm`-gated.** It reaches the host through `VmHostState`,
never through `Vmm`, so a node where `cloud-hypervisor` was uninstalled between
boots still reclaims its survivors — and with no `Vm` registry entry the
supervision set is `Observed(∅)`, so those survivors are authorised rather than
stranded. This is the reason ADR-0082 § A6 keeps artifact removal off the `Vmm`
port.

---

## Alternatives considered

### A1 — Keep `AppState.driver` and `match` on the payload inside the shim

**Rejected.** See D1's third reason: it forces a second representation of "is
the VM capability present" beside the match, and ADR-0022 already committed to
the registry at exactly this trigger. It is also strictly harder to log
honestly — `drivers.kinds()` is the node's advertised capability set.

### A2 — Compose `VmDriver` unconditionally and have it refuse at `start`

**Rejected.** This is SD-5 option B (capability refusal), which Titan withdrew
after finding its evidence inverted: the `EbpfDataplane` precedent it rested on
*does* refuse to boot (`lib.rs:1681-1693` warns **and then**
`return Err(ControlPlaneError::DataplaneBoot(..))`). **There is no in-tree
precedent for a probe that fails and lets the node start.** Composing a driver
in a permanently-refusing state also degrades a *misconfiguration* into "every VM
deploy fails at runtime", burying it exactly as the seven substrate lies bury
themselves.

### A3 — A `[dataplane]`-style `[node] drivers = ["exec", "vm"]` operator knob

**Rejected.** Unstated knobs are out of scope by default (CLAUDE.md), DISCUSS
scoped no node-capability config, and the presence of the hypervisor binary is a
*better* gate than a knob because it cannot disagree with reality. Same shape as
`compose_mtls`.

### A4 — Put the reclamation exclusion in `is_intentionally_stopped`

i.e. widen it to `Operator | SystemGc | PlatformReclaimed`.

**Rejected, and it is the tempting move.** It is DD-1's default 1: `is_restartable`
is that predicate's negation, so widening it makes **every VM on the node stay
dead after an `overdrive serve` restart** — the exact inverse of SD-1's intent,
reached by reusing the nearest existing word. `SystemGc` means *the intent is
gone*; `PlatformReclaimed` means *the intent stands and the platform owes a
replacement*.

### A5 — A new `AllocState::Reclaimed`

**Rejected.** It would force every existing `is_terminal()` and match site to
change and would put a *disposition* in the *lifecycle-bucket* type. Zero new
`AllocState` values is Hera's DD-1 finding, independently re-derived here: the
feature needs a classification **over** terminal states, not a new terminal
state.

### A6 — A separate boot pass, parallel to `adopt_on_restart_recovery`

**Rejected on the race, not on taste.** Both walk the same cgroup tree; two
independent passes race, and the losing order silently leaks a netns. The
boot-epoch drive extends the same boot phase, at the same call site, with a
pinned order.

### A7 — A converge-on-boot (Bar 1) pass, which is what this ADR previously specified

**Rejected by user ruling, 2026-08-11**, and the ruling is not re-litigated here.
Recorded because the *reason* binds DELIVER: the Bar-1-vs-Bar-2 test is *"does
`actual` drift while the system is up?"*, and SD-1's triage answers **yes** —
a clone leaked by a crash between teardown steps, a scope or run directory
stranded by a failed stop (the VM then **unstoppable until the next `serve`
restart**), and SD-2's unbounded-over-lifetime clone leak swept only at the
restart cadence on an appliance with no operator shell. **The argument that does
NOT support Bar 2** — *"the VMM is detached, so `serve` cannot observe its
mid-run exit"* — is **false** and must not be used anywhere: `setsid(2)` detaches
the session and process group, **not parentage** (`driver.rs:355`, `:372-377`),
so the VMM stays a child and the exit watcher's `wait()` fires on any mid-run
death. What forces Bar 2 is the host-state ensemble *around* the process, which
no `wait()` observes.

### A8 — Widen `CgroupFs` with read/list methods instead of a `VmHostState` port

**Rejected on the trait's own contract.** `CgroupFs` is deliberately write-only —
`create_dir` / `write` / `remove_dir` / `probe` / `kind`
(`traits/cgroup_fs.rs:58-257`) — and its `write` postcondition is phrased against
a **hypothetical** read (`:124-128`), i.e. the read side is unexposed by design.
Two of reclamation's three observation surfaces (the VM run root, the staging
directory) are not cgroupfs at all, so widening it would change an established
contract, both adapters and the equivalence test for a consumer that still needs
a second surface walk. `VmHostState` also gives the hydration the **named,
separable seam** SD-1's pin 1 requires so that #197's generalisation is a
refactor rather than a rewrite.

### A9 — A single `ReclaimAllocation { alloc_id, authors_ending: bool }`

**Rejected — DD-5 forbids it, and the reason is the same one DD-2 gives for
`ExitEvent.intentional_stop`.** One command authors an Ending Class and the other
must not; a boolean parameter puts that classification in the **caller's** hands,
which is a sentinel where a sum type belongs (§ *Type-driven design*). It is also
the one failure a test can miss: a collapsed implementation still kills the VMM
and still passes the *"terminal allocation's VMM is killed"* AC, and betrays
itself only against the **row-byte-unchanged** assertion (brief § 105a.10 AC 5).

---

## Consequences

### Positive

- **The feature's pass/fail bar is met by construction**: `lib.rs:1422-1425` no
  longer composes one hardcoded `ExecDriver`, and a real `overdrive serve` +
  `overdrive deploy` reaches the VM driver. Intake precedent warning #1 is
  closed structurally, not by an acceptance criterion.
- SD-5's capability gate and the admission rejection are **the same fact** — a
  missing registry key — so they cannot disagree.
- DD-1's three traps are bound at **four guarded sites** across two reconcilers,
  with an **emission-level** property (P2) — not a hand-maintained site list —
  standing in for the type-level guarantee.
- `classify_driver_failure`'s forward-compatibility parameter is cashed with
  **zero** exec test cases changed, which is Slice 02's falsifiable hypothesis.
- **The Bar-2 reshape costs no design idea and buys a real property.** The
  plan-value split survives verbatim as `plan_reclamation` (pure) plus two
  executors; what changes is that drift is repaired **at the next tick rather
  than the next `serve` restart** — so a scope stranded by a failed stop stops
  being an orphan unstoppable until the next upgrade, which was SD-1's own
  headline failure re-entering by a different door.
- **The kill-authorising predicate fails safe by construction**, not by review:
  `SupervisionSet`'s `Default` is `Unavailable`, so an unpopulated, unhydrated or
  errored discriminator authorises **nothing**. Reading the wrong half of the
  state degrades to "do nothing this tick", never to "kill a live VM".

### Negative, and stated

- **`AllocationSpec`'s shape change touches every construction site**, including
  `workload_lifecycle.rs:743-776`'s `RestartAllocation` spec and the **ten**
  irrefutable `let WorkloadDriver::Exec(..) =` destructures that become
  `match`es — four in production (`aggregate/mod.rs:940`, `:1020`;
  `workload_lifecycle.rs:742`, `:856`) and six in tests. All compiler-enforced,
  none silent — but it is the widest mechanical change in the feature.
  *(Counted at review iteration 2; the first draft said eleven.)*
- **`ParseError::MissingExec` is deleted**, and any test or fixture asserting on
  it changes in the same PR. Per intake I-5's single-cut ruling there is no
  alias and no grace period.
- **The Ending Class is carried by three predicates, not one type.** Totality is
  a tested property rather than a compile-time guarantee; a fourth class would be
  the trigger to mint `EndingClass`. And the binding is now **four** sites in
  `WorkloadLifecycle` plus one in `ServiceLifecycle`, not three — which is
  itself the argument for property **P2** over a hand-maintained site list.
- **Replacing `AppState.driver` is a four-seam change, not a one-line change**
  (§ D2a): the composition root, `exit_observer`'s spawn topology, the shim's
  stop/terminal arms (which need a new `alloc_drivers` index because those
  `Action`s carry no spec), and `MtlsInterceptWorker`'s per-alloc gate. The
  first draft specified only the first and would have shipped a VM that starts,
  cannot be stopped, and whose exit is never observed.
- **`alloc_drivers` is in-memory and per-boot.** That is correct rather than
  lossy — after a `serve` restart there are no live VM allocations to stop,
  because the boot-epoch drive has already reclaimed them (SD-1) — but it is a
  second piece of per-boot driver state beside the registry, and a third driver
  is the trigger to move to `workload_id` on the `Action` variants instead.
- **The boot-epoch drive kills every running VM on every `serve` restart** —
  ~1.1 s of re-boot per VM on metal, against a process respawn that is orders of
  magnitude cheaper. Accepted because the alternatives are "the exit status
  becomes a lie" (adopt) or "GiB-scale unstoppable orphans" (do nothing).
- **Installing `cloud-hypervisor` can flip a node from booting to refusing to
  boot** (D2's inverse hazard).
- **The Bar-2 ruling makes this feature the FIFTH site awaiting
  [#197](https://github.com/overdrive-sh/overdrive/issues/197)'s shared
  host/node-infrastructure reconciler model** (with #198 / #199 / #234), and SD-1
  deliberately does **not** found that abstraction here. The real risk is that
  this shape gets copy-pasted before anyone generalises it; the mitigation is a
  design obligation rather than a hope — `VmHostState::observe()` is a named,
  separable step returning a plain value, so the generalisation is a refactor of
  an existing seam. It is also the first in-tree reconciler whose `actual` comes
  from **host state** rather than the intent or observation stores, so #197
  inherits a worked example of exactly the hydration problem it exists to solve.
- **A new reconciler is five enums and four matches** (`AnyReconciler`,
  `AnyState`, `AnyReconcilerView`, `AnyViewMap`, plus `register`,
  `hydrate_desired`, `hydrate_actual` and `AnyReconciler::reconcile`) — all
  compiler-enforced, none silent, and enumerated in brief § 105a.9 so DELIVER
  does not discover them mid-slice.
- **`spawn_convergence_loop` gains a periodic submission**, which is the one
  place this feature touches shared convergence machinery. Without it the
  reconciler would never tick: the broker is event-driven, has no bootstrap
  sweep, and nothing in tree names `vm-reclamation`. The cost is a
  three-surface walk twice a minute per node.
- **`Driver` gains one defaulted method** (`live_allocations`), exercising intake
  I-2's licence that the first two drafts recorded as deliberately unexercised.
  Minimal and fail-safe (`None` ⇒ authorises nothing), but it is a change to the
  trait every driver implements.

### Neutral

- `DriverType::MicroVm` is deleted and `DriverType::Vm` survives (intake I-5).
  Both variants already exist (`traits/driver.rs:43`, `:45`); the deletion costs
  two exhaustive-match arms, an OpenAPI regeneration (`DriverType` derives
  `ToSchema` transitively through `TransitionSource`), and an amendment to the
  now-false *"existing variants never change their wire form"* docstring at
  `traits/driver.rs:26-29`. `DriverType` reaches **no** persisted row, so there
  is no rkyv envelope bump for it.
- Adding `WorkloadDriver::Vm` **is** an rkyv event — `JobEnvelope` V1 → V2 via
  the full six-step single-commit procedure, user-ruled at intake I-5. Existing
  golden fixtures are never touched.
