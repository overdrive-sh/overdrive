# ADR-0083 — `DriverRegistry` replaces the single `AppState.driver`; `AllocationSpec` carries a per-driver payload; the registry *is* the VM capability gate

## Status

Accepted. 2026-08-11. **Revised 2026-08-11 (review iteration 3)** — § D7 is
rewritten from a converge-on-boot (Bar 1) pass to a registered `Reconciler`
(`reconcilers.md` **Bar 2**) **by user ruling**; A7–A9 added; `plan_vm_reap` /
`execute_vm_reap` / `VmReapPlan` **deleted**. **Amended 2026-08-11 (review
iteration 4, NEW-1 / NEW-3)** — § D7 gains item **2a**, the supervision handle's
lifecycle per Hera's **DD-1(b.i)**: it is a claim on *authoring an ending*, so
`Driver` gains a **second** defaulted method (`release_supervision`) and the
Consequences' "one defaulted method" line is corrected to two. No prior decision
is reversed; the addition pins a lifecycle the earlier text left open. The
application-architecture shape is pinned in `brief.md` § *Application
Architecture* → **§ 105a**. **Amended 2026-08-11 (fold-in of prerequisite
D-3, same DESIGN pass, user ruling)** — § D5's Cause-variant table gains a
**thirteenth** row, `VmOutOfMemory { limit_bytes, oom_kill_count }`,
constructed in **Slice 01** (not 02–04, since it is built alongside the
mid-run exit watcher that slice already owns) from the new
`CgroupAccounting` port's read — see companion ADR-0082 § D8 for the port,
the `ExitEvent.oom` field it threads through, and the reconciliation with §
A8's already-rejected "widen `CgroupFs`" alternative. No prior row is
renumbered or reworded. **Amended 2026-08-11 (DISTILL-surfaced gap 1 — a
mid-run storage-daemon death had no Cause variant), same DESIGN pass** — §
D5's Cause-variant table gains a **fourteenth** row, `VmStorageDaemonDied {
socket, exit_code, signal }`, constructed in **Slice 04** from a new
`ExitEvent.storage_daemon_died` field mirroring row 13's `oom` field
(ADR-0082 § D8) — checked AHEAD of `ExitKind` entirely, not nested inside a
`Crashed` arm the way row 13 is. This rules the hedge S-VM-65 recorded
against this table
(*"`VmGuestMountFailed`'s sibling variant, or a distinct mid-run variant"*):
it is a distinct variant; `VmGuestMountFailed` stays scoped to the
guest-reported START-time mount failure. No prior row is renumbered or
reworded. **Amended 2026-08-11 (DISTILL-surfaced gap 2 — the `SimVmm`
injection seam `brief.md` § 100 assumed but never wired), same DESIGN
pass** — § D8 is added: `ServerConfig.vmm_override`, a
`#[cfg(feature = "integration-tests")]`-gated whole-port substitution seam
for `Vmm`, pinning how S-VM-13 and S-VM-51 reach `SimVmm` inside a real
`overdrive serve`. Ruled explicitly NOT the seam that reaches S-VM-67 —
`virtiofsd`'s sandbox check sits outside the `Vmm` port entirely, and that
boundary is stated rather than papered over. **Amended 2026-08-11 (user
ruling, closes § D8's S-VM-67 open item), same DESIGN pass** — § D8's
closing section is amended: the user ruled **path (b)** of its own two
named paths. The `--sandbox=namespace` posture is verified at the
**launch-argument construction layer** — the same enforcement tier ADR-0082
§ D2.1 already uses for `image_type=raw` (private fields, one rendering
site, a pure unit test on the rendered value, a lint clause forbidding a
second construction site) — never through a real `overdrive serve`.
**No storage-daemon supervision port is minted by this feature.** Path (a)
is explicitly not taken. No prior decision is reversed; this closes the one
item § D8 deliberately left open.
**Amended 2026-08-14 (DWD-23, closes 01-09 review finding D2 — implementation
status, no decision reversed).** §§ D1.3 / D2 / D4's *"`[vm]` rejected at
**admission** naming the absent capability"* is ratified design intent that
**remains in force and is NOT amended**. Recorded for the reader: step 01-09
shipped the **dispatch-time fallback only** (the `drivers.get(kind) → None` arm in
`action_shim`, SAFE — the deploy is *admitted* and the alloc then reaches `Failed`
naming the capability, S-VM-12); the *admission-time* gate in
`handlers.rs::submit_workload` was never in 01-09's `implementation_scope`
(`lib.rs` / `cgroup_accounting` / `exit_observer` — not `handlers.rs`) and is
scoped to a **follow-up step, pending user build-vs-defer approval**. The gate is
cheap — `AppState.drivers: Arc<DriverRegistry>` and `DriverRegistry::{supports,
kinds}` (whose doc already reads *"iterated for the admission-rejection message"*)
have existed since step 01-08. The dispatch-time fallback stays as multi-node-ready
defense-in-depth. `brief.md` § 104 carries the same status note; see
`distill/wave-decisions.md` DWD-23.
**Amended 2026-08-16 (DWD-24, DELIVER Phase-03 upstream resolution).**
§ D5 retires `classify_driver_failure(text, driver, command)` and pins the
exact typed `DriverStartFailure` / `DriverStartClass` /
`ExecStartFailure` / `VmStartFailure` API. Exec's existing operator-visible
classes and verbatim detail are preserved; VM causes are constructed where
their fields are known; every unknown uses the existing
`DriverInternalError` fallback. Rows 13/14 remain exit-observer-only, and row
15 is appended for the already-known post-READY `EXEC` delivery failure.
**Amended 2026-08-18 (user ruling — volumes cut from this feature; §§ D3 / D5 / D8
volume decisions superseded).** `[[vm.volume]]` was designed as a **virtiofs host↔guest
bind-mount** (a host `source` directory shared into the guest at `target`, host and guest
seeing the same bytes). The user ruled this the **wrong mechanism and the wrong name** and
**removed volumes from this feature entirely.** A real persistent volume is guest-owned and
**block-device-shaped** (`overdrive-fs`, GH #97, Phase 6.13 — whose own spike evidence
P9–P11 says the guest sees a block device, ext4 / vhost-user-blk, NOT a virtiofs / FUSE
mount, the *opposite* mechanism), and #97 was already scoped OUT of #42 (user ruling
2026-08-10). The `virtiofsd` mechanism itself is GH #43's ([3.6] virtiofsd lifecycle +
cross-workload sharing). US-VM-8's host-`ls` / `cat` use case is fictional on the production
target (immutable appliance OS, no operator shell, node-local artifact not surviving a
reschedule; the correct sink is the object store Garage #22 or `overdrive-fs` #97). Volumes
are therefore **deferred to #97 / #43 / #22, not renamed or kept.** The superseded items are
enumerated in the **Amendment 2026-08-18** section at the end; the volume-free VM
boot / stop / restart / confinement path (§ D5 rows 1–7, 13, 15; the `ExitEvent.oom` field;
§§ D1 / D2 / D2a / D4 / D6 / D7 / D8-`vmm_override`) is UNCHANGED.
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
binding half of § *Domain Model* → **DD-1** / **DD-1(b)** / **DD-1(b.i)** /
**DD-5** (*"no reconciler may author a terminal claim on a Platform-Reclamation
row"*, the supervision precondition, **the handle's lifecycle and its
corollary**, and the two-`Action` split with its payload prohibitions). None of
those sections is amended; all are consumed.

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

**A miss broadcasts the stop/terminal call to every composed driver — the driver
that owns the alloc is always in that set, so this is *not* wrong-driver routing
and strands no orphan.** *(Amended 2026-08-14 by user ruling — DWD-22 / GH #42,
closing the 01-08 review's MAJOR finding D1. The first draft pinned a typed
`ShimError::UnknownDriverForAlloc { alloc_id }` here. That verdict rested on a
strawman fallback the shipped `resolve_drivers_for_alloc`
(`action_shim/mod.rs:668-678`) never implemented, and — taken literally — would
have inverted its own safety goal. The variant is **retired, not implemented**;
it never existed in the tree.)*

On a miss, `resolve_drivers_for_alloc` returns **all** composed drivers and the
shim fires the stop/terminal call on each. This is safe, and it is *stronger*
than the typed error the strawman rejected, for two reasons:

1. **Broadcast is not "silently routed to `ExecDriver`."** The rejected fallback
   was *pick the default (exec) driver* — which would no-op `stop` on a VM alloc
   (`DriverError::NotFound`, swallowed by `let _ =` at `:1859`) and strand the
   GiB-scale orphan SD-1 exists to prevent. Broadcast routes to the **`VmDriver`
   too**, so a VM alloc's `stop` reaches the driver that owns it — the orphan is
   *never* stranded. Every `Driver::stop` / `on_alloc_stable` /
   `on_alloc_terminal` is documented NotFound-tolerant / no-op for an alloc it
   does not track, so the fan-out to non-owning drivers is harmless. **That
   NotFound-tolerant/no-op property is now load-bearing** — it is what makes the
   broadcast safe, and it must hold for every driver this registry composes.
2. **The index is in-memory and per-boot, so a miss is a legitimate expected
   state, not a bug.** It arises whenever `dispatch` acts on an alloc not started
   in the current boot epoch — an operator `stop` of a workload that has been
   `Running` since before a `serve` restart (freshly-empty index), or a driver
   lifecycle hook exercised directly in a test
   (`stable_does_not_stop_probe_supervision.rs` calls `on_alloc_running` without
   a `StartAllocation` dispatch, so the index carries no entry). Returning
   `Err(UnknownDriverForAlloc)` on such a miss would route the stop to **nobody**
   and create the very orphan the typed error meant to prevent. A typed error is
   correct only if a miss is *always* a bug; here it is not — and the shim cannot
   tell a same-boot miss from a cross-boot one without a `boot_epoch` flag,
   precisely the self-declared-boolean check § D7 item 1 rejects on the
   kill-authorising path for the same reason.

**Diagnostic residual (recorded, non-blocking).** Broadcast does mask a genuine
*same-boot* miss (an index entry lost to a real defect) that a hard error would
surface. The blanket typed error cannot recover that without the rejected epoch
flag; the honest recovery is observability, not refusal — an optional
`tracing::debug!(name: "shim.alloc_driver.index_miss", %alloc_id)` on the
fallback arm makes it non-silent without changing routing. It is **not** required
for conformance.

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
    // SUPERSEDED 2026-08-18 (volumes cut — see Amendment 2026-08-18): this field is
    // REMOVED; `VmPayload` carries no volume field, and the `VmVolume` value is deleted.
    // Deferred to GH #97 (block-device managed volume) / #43 (virtiofsd) / #22 (object store).
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

### D5 — drivers author typed start causes; the shim converts, never classifies text

`classify_driver_failure(text, driver, command)` is retired. The action shim
must not branch on `DriverType`, inspect `Display`, parse a path, or match an OS
error sentence. The exact public boundary in `overdrive-core` is:

```rust
pub enum DriverError {
    StartRejected { failure: DriverStartFailure },
    // NotFound / Io / NetnsEntry remain unchanged.
}

pub struct DriverStartFailure {
    pub class: DriverStartClass,
    /// Non-empty verbatim low-level diagnostic, preserved in
    /// AllocStatusRow.detail. Never a classification input.
    pub detail: String,
}

#[non_exhaustive]
pub enum DriverStartClass {
    Exec(ExecStartFailure),
    Vm(VmStartFailure),
    Unclassified { driver: DriverType },
}

#[non_exhaustive]
pub enum ExecStartFailure {
    BinaryNotFound { path: String },
    PermissionDenied { path: String },
    BinaryInvalid { path: String, kind: String },
    CgroupSetupFailed { kind: String, source: String },
}

#[non_exhaustive]
pub enum VmStartFailure {
    KernelNotFound { path: String },
    RootfsNotFound { path: String },
    HypervisorAbsent { searched: Vec<String> },
    BootDeadlineExceeded {
        deadline_ms: u64,
        console_tail: Option<String>,
    },
    KernelFormatUnsupported {
        path: String,
        arch: String,
        detail: String,
    },
    ConfinementUnavailable {
        control: ConfinementControl,
        detail: String,
    },
    GuestExitUnreported {
        vmm_exit_code: Option<i32>,
        vmm_signal: Option<u8>,
    },
    // SUPERSEDED 2026-08-18 (volumes cut — Amendment 2026-08-18): the five volume
    // variants immediately below are REMOVED (deferred to #97 / #43 / #22).
    VolumeSourceNotFound { path: String },
    StorageDaemonAbsent { searched: Vec<String> },
    GuestMountFailed { target: String, detail: String },
    StorageSocketTimeout { socket: String, waited_ms: u64 },
    StorageSandboxUnavailable { requested: String, detail: String },
    // GuestCommandDispatchFailed is NOT a volume variant (row 15) — it STAYS.
    GuestCommandDispatchFailed { detail: String },
}

pub enum ConfinementControl {
    Landlock,
    Seccomp,
    UidDrop,
    RlimitFsize,
    RlimitNofile,
    KvmAccess,
}

impl From<&DriverStartFailure> for TransitionReason;
```

`DriverStartClass` is the family discriminator; there is deliberately no
independent `driver` field on `StartRejected` that could contradict it. The
`Unclassified` arm retains a `DriverType` only for a failure with no named
class. Its conversion is always
`DriverInternalError { detail: failure.detail.clone() }`.

The conversion is total and one-to-one for known causes:

| Typed Exec cause | Exact `TransitionReason` |
|---|---|
| `Exec(BinaryNotFound { path })` | `ExecBinaryNotFound { path }` |
| `Exec(PermissionDenied { path })` | `ExecPermissionDenied { path }` |
| `Exec(BinaryInvalid { path, kind })` | `ExecBinaryInvalid { path, kind }` |
| `Exec(CgroupSetupFailed { kind, source })` | `CgroupSetupFailed { kind, source }` |

The canonical Exec payload strings are preserved from the live operator
surface: ENOEXEC uses `kind == "exec_format_error"`; cgroup setup uses only
`"create_scope"` or `"place_pid"`. ENOENT, EACCES, and ENOEXEC are selected
from structured OS error identity at `ExecDriver`, not from English `Display`
text. Existing row `detail` remains the verbatim diagnostic. **Zero existing
Exec operator-visible classifications change.**

The VM mapping is:

| # | `VmStartFailure` / mid-run source | Exact `TransitionReason` payload | Slice | Notes |
|---|---|---|---|---|
| 1 | `KernelNotFound { path }` | `VmKernelNotFound { path: String }` | 02 | Per-allocation path reopen; post-composition deletion is observable |
| 2 | `RootfsNotFound { path }` | `VmRootfsNotFound { path: String }` | 02 | Exact configured master path |
| 3 | `HypervisorAbsent { searched }` | `VmHypervisorAbsent { searched: Vec<String> }` | 02 | Spawn-time `NotFound` only; names every searched path |
| 4 | `BootDeadlineExceeded { deadline_ms, console_tail }` | `VmBootDeadlineExceeded { deadline_ms: u64, console_tail: Option<String> }` | 02 | Tail comes from `VmmDiagnostics`, never timeout text |
| 5 | `KernelFormatUnsupported { path, arch, detail }` | `VmKernelFormatUnsupported { path: String, arch: String, detail: String }` | 02 | **C-7.** Stable validator diagnosis; never "size cap" |
| 6 | `ConfinementUnavailable { control, detail }` | `VmConfinementUnavailable { control: ConfinementControl, detail: String }` | 03 | One typed cause class, six controls |
| 7 | `GuestExitUnreported { vmm_exit_code, vmm_signal }` | `VmGuestExitUnreported { vmm_exit_code: Option<i32>, vmm_signal: Option<u8> }` | 03 | Boot-race `VmmExit`; stderr remains outer row `detail` |
| 8 | `VolumeSourceNotFound { path }` | `VmVolumeSourceNotFound { path: String }` | 04 | `VmDriver`, outside `Vmm` |
| 9 | `StorageDaemonAbsent { searched }` | `VmStorageDaemonAbsent { searched: Vec<String> }` | 04 | `VmDriver`, outside `Vmm` |
| 10 | `GuestMountFailed { target, detail }` | `VmGuestMountFailed { target: String, detail: String }` | 04 | Guest-reported start-time mount failure |
| 11 | `StorageSocketTimeout { socket, waited_ms }` | `VmStorageSocketTimeout { socket: String, waited_ms: u64 }` | 04 | `VmDriver`, outside `Vmm` |
| 12 | `StorageSandboxUnavailable { requested, detail }` | `VmStorageSandboxUnavailable { requested: String, detail: String }` | 04 | `VmDriver`, outside `Vmm` |
| 13 | mid-run `ExitEvent.oom` | `VmOutOfMemory { limit_bytes: u64, oom_kill_count: u64 }` | 01 | Exit observer only; never `DriverStartFailure` |
| 14 | mid-run `ExitEvent.storage_daemon_died` | `VmStorageDaemonDied { socket: String, exit_code: Option<i32>, signal: Option<u8> }` | 04 | Exit observer only; checked ahead of `ExitKind` |
| 15 | `GuestCommandDispatchFailed { detail }` | `VmGuestCommandDispatchFailed { detail: String }` | 02 | Appended: READY arrived but `EXEC` delivery failed before `Running` |

Rows 1–12 and 15 are start-time causes and may cross
`DriverError::StartRejected`. Rows 13 and 14 are mid-run facts and MUST NOT.
Not every VM start cause originates in `VmmError`: kernel/rootfs preflight,
the boot deadline, `VmmExit`, guest control, and storage-sidecar facts are
owned by `VmDriver`; ADR-0082 D1.1 names the smaller typed `VmmError` subset.

**Why #6 is one variant and not six**, against US-VM-2's *"no two distinct
causes share a variant"*: the distinct causes there are all *"this host cannot
supply confinement control X"* — one cause class discriminated by a typed field,
which is what Slice 03 asks for in as many words (*"a **fifth** variant minted
in Slice 02's shape"*). A `String` discriminant would have been the stringly-
typed version and is rejected.

> **SUPERSEDED 2026-08-18 — volumes cut (Amendment 2026-08-18).** Row 14
> (`VmStorageDaemonDied`), the `ExitEvent.storage_daemon_died` field, the
> `StorageDaemonDeathFacts` value, and this ahead-of-`ExitKind` precedence rule are
> **removed** with volumes; `ExitEvent` keeps only its `oom` field (row 13). Deferred to
> #43 (virtiofsd lifecycle). The prose below is retained as history, not as live design.

**Row 14 — why it is mid-run and not `DriverStartFailure`'s to construct,
and why it must be checked ahead of `ExitKind`, not nested inside it.**
*(2026-08-11, gap-closure amendment; closes the hedge S-VM-65 recorded:
"`VmGuestMountFailed`'s sibling variant, or a distinct mid-run variant per
ADR-0083 § D5".)* `DriverError::StartRejected` carries a **start-time**
failure. A `virtiofsd` death mid-run is not a start failure; the allocation is
already `Running`,
and by the time the platform observes the death there is no `create()` call
in flight to reject. The construction mechanism is instead the same shape
ADR-0082 § D8 introduced for row 13: `ExitEvent` gains a second additive
field,

```rust
pub struct StorageDaemonDeathFacts {
    pub socket:    String,       // VmRunDir::vsock_socket-shaped path, i.e.
                                  // the virtiofsd UDS this VM's daemon served
    pub exit_code: Option<i32>,
    pub signal:    Option<u8>,
}

// ExitEvent (traits/driver.rs) gains, alongside `oom` (ADR-0082 § D8):
pub storage_daemon_died: Option<StorageDaemonDeathFacts>,
```

set by `VmDriver`'s own mid-run supervision of the daemon it spawned
directly (system constraint 9 / US-VM-9 — `virtiofsd` is a **sidecar
process `VmDriver` supervises itself**, the same shape `ExecDriver` already
uses for its own workload process; it is not behind the `Vmm` port — see §
D8 below for why that matters to Gap 2). The field is set **only** when the
daemon's exit was observed BEFORE the workload itself reported an outcome
and BEFORE teardown began — the exact before/during-teardown guard US-VM-9
AC 1 / AC 2 name as the discriminator, mirroring `oom`'s own "immediately
after exit and before any teardown" gating (ADR-0082 § D8). A daemon that
exits as part of ordinary teardown, after the guest's own outcome is
already reported, leaves this field `None` — its exit is still observed
(audit trail, US-VM-9 AC 1) but is not this fact.

`overdrive-control-plane`'s `worker::exit_observer::handle_exit_event`
(already touched by ADR-0082 § D8's `oom` precedence check) gains a
**second** additive precedence check, checked **before** the existing
`ExitKind` match runs at all — not nested inside the `Crashed` arm the way
row 13's check is:

```
event.storage_daemon_died.is_some()
    → TransitionReason::VmStorageDaemonDied { socket, exit_code, signal }  // row 14, checked FIRST
ExitKind::Crashed { .. } if event.oom.is_some_and(|o| o.oom_kill_count > 0)
    → TransitionReason::VmOutOfMemory { .. }                              // row 13, unchanged
ExitKind::Crashed { exit_code, signal }
    → TransitionReason::WorkloadCrashedImmediately { .. }                 // unchanged default
ExitKind::CleanExit { .. }
    → (unchanged mapping)
```

**Why ahead of `ExitKind`, stated because getting this wrong reproduces the
feature's own headline defect one phase later.** A guest whose writes start
silently failing after its storage daemon dies has no reason to notice —
per `[D4]`, `overdrive-init` execs the operator's command and waits on it;
it does not validate the operator's own I/O. That guest can still exit `0`
and report `EXIT 0` over the beacon, which resolves `ExitKind::CleanExit`.
If the daemon-death check were reached only from inside a `Crashed` arm (row
13's shape), a guest that self-reports success after its share died would
resolve `CleanExit` first and the daemon-death fact would never be
consulted — silently reproducing `VmGuestMountFailed`'s composite-lie defect
(row 10, § D2.4 of ADR-0082) one execution phase later, and exactly the
"job wrote 40 frames and the share died... job reports success anyway"
failure US-VM-9's Problem statement names. Checking
`storage_daemon_died.is_some()` first — overriding a guest-reported
`CleanExit`, never merely supplementing a `Crashed` — is what makes US-VM-9
AC 1/2's single discriminated classification (not three independent checks)
actually hold across both the `Crashed` and `CleanExit` guest-reported
outcomes, not only the `Crashed` one.

**Fifteen distinct causes**: thirteen start-time classes (rows 1–12 and the
append-only row 15) plus two mid-run classes (`VmOutOfMemory`, row 13, and
`VmStorageDaemonDied`, row 14), against K3's "≥ 4 distinct". Per DD-3, the
reclamation **disposition** is deliberately **not** in this table and must not
be counted toward K3 — counting a disposition as a failure cause would let the
feature satisfy K3 without shipping a fourth diagnosis.

> **SUPERSEDED 2026-08-18 — volumes cut (Amendment 2026-08-18).** With volumes removed,
> the count is **nine** distinct VM diagnoses — rows 1–7 and row 15 (start-time) plus row
> 13 `VmOutOfMemory` (mid-run) — still comfortably ≥ 4. The five volume start-time causes
> (**rows 8–12**) and mid-run **row 14** (`VmStorageDaemonDied`), plus the enum's five
> volume variants above, are superseded. Deferred to #97 / #43 / #22.

`TransitionReason` is `#[non_exhaustive]` (`transition_reason.rs:87`) and every
addition is appended, preserving rkyv discriminants — row 15 therefore stays
physically after rows 13/14 despite being a start-time cause. The same
discipline is stated verbatim for `StoppedBy` at `:237-241`.

**Enforcement.** Rust exhaustiveness is the first layer: the core-owned
`From<&DriverStartFailure>` match names every current nested variant, so an
additive class cannot compile until its transition mapping is explicit. The
structural layer is the ADR-0032 `xtask::dst_lint` rule rejecting the retired
`reason: String` field and `classify_driver_failure` function. The behavioral
layer is (a) a table test over every Exec/VM class and payload, (b) unchanged
Exec operator assertions including `kind == "exec_format_error"`, and (c) the
S-VM acceptance scenarios reading `workload describe`, including the
`DriverInternalError` unknown fallback. These layers answer different
questions; no text convention is trusted as a contract.

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
2a. **The handle is a claim on *authoring an ending*, not a grip on a running
   process** (Hera's **DD-1(b.i)**; added 2026-08-11, iteration-2 review NEW-1 /
   NEW-3). It is held from the first line of `VmDriver::start` — before the run
   directory exists — until that ending has been **authored** (the terminal row
   written) or authorship **abandoned as impossible**; never released at process
   death, at the exit watcher's return, or while an exit report is in flight.
   The claim therefore carries a **phase** (`Held` → `EndingInFlight`), both
   phases report as supervised, the watcher's `ExitEvent` emission is gated on an
   **atomic** hand-off transition, and `Driver` gains a second defaulted sync
   method **`release_supervision(&self, alloc: &AllocationId)`** whose callers are
   the exit observer (once per `ExitEvent`, on **every** `RetryOutcome` arm — the
   abandonment boundary) and every shim arm that writes a terminal row (after the
   write resolves `Ok`). Two further consequences are pinned rather than left to
   DELIVER: `execute_reclaim_allocation`'s existing prior-row re-read becomes a
   **terminality guard over the whole executor**, and `hydrate_actual` reads
   `observe()` **first** and the supervision set **last**. **Brief § 105a.3 is the
   SSOT** for the transition table, the abandonment boundary and the residual;
   this item records the decision, not its mechanics.
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
`Clock` so the cadence is DST-controllable rather than wall-clock. **The mechanism
and the value were both ratified by the user on 2026-08-11**; the constant is
compile-time, **not operator-tunable, and no knob is promised**. The interval
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

> **Corrected by the 2026-08-17 (second) amendment, § D7 correction item 1;
> clone location further updated by the ADR-0082 fourth amendment (2026-08-18).**
> There is no single "image directory" after § D3a. Per-launch clones land in the
> platform-owned `clone_staging_dir(data_dir)` on the master's (VM data)
> filesystem (ADR-0082 fourth amendment § (c-fix.2) — see § D3a's amendment note;
> the earlier "beside the operator's own `[vm] rootfs`" location is stale). Read
> the third surface as: *every per-launch clone reachable through the
> platform-owned clone index (§ D3f)* — which is **unchanged**, because the index
> records the clone's location wherever it lives. The model is still **three**
> surfaces; the index is the index *for* the third, not a fourth.

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

> **Corrected by the 2026-08-17 (second) amendment, § D7 correction item 2.**
> This paragraph, and the clause above it naming the clone *"the only surface
> that survives a host reboot"*, were true as authored and were falsified by
> § D3a. Between § D3a and that amendment they were false in **both**
> directions at once: the enumerated clone directory was `/run` (tmpfs — does
> not survive a reboot, and per ADR-0082 § D2 gap 3 cannot hold a clone at all,
> since staging into `/run` fails `EXDEV`), while the clones that genuinely do
> survive a reboot sat unenumerated on the operator's persistent filesystem.
> Both statements hold again under § D3f–D3h: the index is durable
> (`data_dir`), and the index link's lifetime strictly contains the clone's.
> The filename-carries-attribution rule this paragraph gives is unchanged and
> is what the index still keys on.

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

**Closed 2026-08-11.** SD-1 pin 5 was revised in `brief.md` § *System
Architecture* to assert the property rather than the literal mechanism, and to
name registration as inert with the spawn ordering as the constraint — so this is
a settled agreement between the two sections, not an outstanding divergence. The
`&mut self` / `Arc::new(runtime)` reason above is kept on purpose: it is what a
later reader needs when they wonder why registration order is not the constraint.

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

### D8 — the `Vmm` fault-injection seam: `ServerConfig.vmm_override`, and the boundary it does NOT reach

> **Added 2026-08-11, gap-closure amendment.** § D2's composition-root
> snippet named no override seam, but `brief.md` § 100 already asserted
> *"`SimVmm` is the injection point for Slice 03's fail-closed confinement
> case"* and three DISTILL scenarios — S-VM-13 (non-reflink staging),
> S-VM-51 (confinement-unavailable), and, only partially (see below),
> S-VM-67 (storage-sandbox-unavailable) — are written against a `SimVmm`
> "injected at the port boundary" (system constraint 1) inside a REAL
> in-process `overdrive serve`. The fixed Lima kernel test envelope cannot
> organically produce a non-reflink staging filesystem, an absent
> `--landlock` flag, or an unreachable `/dev/kvm` (ADR-0082 § D5's own
> fault-injection table); left unpinned, a crafter improvises the wiring —
> forbidden by CLAUDE.md § "Implement to the design". `dataplane_override`
> is precedent for the PATTERN'S existence in this codebase, never
> authorisation for its SHAPE — see the ruling below, which is why this
> seam is not shaped like it.

**The seam.** `ServerConfig` (`overdrive-control-plane/src/lib.rs`) gains a
field in the same family as the existing `mtls_identity_override` (a
whole-port-implementation swap, `Arc<dyn Trait>`) — **not** the
`dataplane_probe_fault` shape (a one-shot forced-message seam layered on
the REAL adapter via a setter), because `Vmm` needs multiple **distinct
typed** fault classes across **two** methods (`probe` and `create`), which
a single `Option<String>` cannot carry, and because `VmmProbeError` /
`VmmError` cannot derive `Clone` (both embed `std::io::Error`, the same
reason `dataplane_probe_fault` reached for a `String` in the first place —
this seam does not have that escape available since it needs the type
distinction, not just a message):

```rust
// crates/overdrive-control-plane/src/lib.rs, ServerConfig

/// Test-only adapter-substitution seam for the `Vmm` port. When `Some(v)`,
/// `compose_production_driver` uses THIS adapter in place of
/// `CloudHypervisorVmm::discover`'s result, so a real in-process
/// `overdrive serve` runs the SAME discover → probe → insert sequence
/// (§ D2) against a `SimVmm` carrying an injected fault, rather than
/// against the real hypervisor binary. `None` in production; the field
/// does not exist in a production binary (`#[cfg(feature =
/// "integration-tests")]`-gated on both the declaration and its one use
/// site, mirroring `mtls_identity_override`'s discipline).
#[cfg(feature = "integration-tests")]
pub vmm_override: Option<Arc<dyn overdrive_core::traits::vmm::Vmm>>,
```

**Placement — § D2's snippet, amended.** The `match
CloudHypervisorVmm::discover(&vm_layout).await` resolves the override
first, and every line after that resolution is **unchanged**:

```rust
#[cfg(feature = "integration-tests")]
let discovered = match &config.vmm_override {
    Some(injected) => Ok(Some(Arc::clone(injected))),
    None => CloudHypervisorVmm::discover(&vm_layout).await
        .map(|found| found.map(|v| Arc::new(v) as Arc<dyn Vmm>)),
};
#[cfg(not(feature = "integration-tests"))]
let discovered = CloudHypervisorVmm::discover(&vm_layout).await
    .map(|found| found.map(|v| Arc::new(v) as Arc<dyn Vmm>));

match discovered {
    Ok(None) => { /* capability absence — unchanged */ }
    Ok(Some(vmm)) => {
        // Earned Trust is UNCONDITIONAL here: `.probe()` runs against
        // WHATEVER adapter is present, production or injected. There is
        // no `if fault_injected { skip probe }` branch anywhere in this
        // function — the composition root calls the same trait method
        // either way.
        if let Err(source) = vmm.probe().await { /* unchanged */ }
        drivers.insert(Arc::new(VmDriver::new(vmm, clock, fs, cgroup_accounting, vm_layout)));
    }
    Err(source) => { /* unchanged */ }
}
```

Every downstream consumer — `DriverRegistry`, § D2a's per-driver
`exit_observer` loop, `alloc_drivers`, `MtlsInterceptWorker`'s
`DriverType::Exec` gate, `VmReclamation` — sees `Arc<dyn Vmm>` and is
**unchanged and unaware the seam exists**. That is what "port-boundary
substitution" means structurally: one binding changes; nothing downstream
of it does.

**Why this is legitimate, and why it is NOT the #248 / `dataplane_override`
shape** (`.claude/rules/development.md` § "Ground the premise: a state
only a test seam can produce is not a feature"):

1. **The states it produces are production-reachable.** `ReflinkUnsupported`
   / `LandlockFlagAbsent` / `LandlockLsmAbsent` / `KvmUnreachable` /
   `RunDirUnusable` are ADR-0082 § D5's own named, catalogued substrate
   lies — a staging filesystem without reflink, a CH build without
   `--landlock`, a kernel without the Landlock LSM are real host
   conditions. The seam does not invent a state; it makes an
   already-real state reachable on the one Lima kernel the envelope runs.
   Contrast #248: `workload_addr = None` occurred **only** when
   `dataplane_override` disabled mTLS composition entirely — a state no
   real deploy with mTLS composed could ever reach.
2. **It does not gate off a subsystem.** `dataplane_override` flips
   `compose_mtls = dataplane_override.is_none()` — a whole layer stops
   composing. `vmm_override` composes nothing differently: the registry,
   `VmDriver`, the exit-observer loop and `VmReclamation` all run exactly
   as in production, against whichever `Arc<dyn Vmm>` is bound. Only the
   one port binding differs — the same shape as `mtls_identity_override`
   (the mTLS layer stays fully composed; only `IdentityRead`'s source
   changes) and structurally narrower than `dataplane_override`.
3. **`probe()` still runs, unconditionally.** Wire → probe → use
   (principle 13) holds against whichever adapter is bound; a
   hand-installed bypass that skipped `.probe()` to force the refusal
   would be the CLAUDE.md-forbidden shape. Calling the real trait method
   against a `SimVmm` configured to answer honestly with an injected
   fault is exactly what a `Sim*` adapter at a port boundary is for
   (system constraint 1, verbatim).
4. **Production cannot construct the state.** The field is
   `#[cfg(feature = "integration-tests")]`-gated on both the declaration
   and the composition-root use site — a production binary contains
   neither the field, the branch, nor a code path that could read it.

**Ruling: this is the port-trait-boundary pattern
(`.claude/rules/development.md` § "Port-trait dependencies"), not a
composition-root override in the #248 sense.**

**What the seam does NOT reach — S-VM-67, stated plainly rather than
glossed.** `Vmm::create(&VmConfig)` spawns **"ONE confined hypervisor
process"** (ADR-0082 § D1, verbatim), and `VmConfig` (ADR-0082 § D2)
carries no volume field at all. `virtiofsd` is never composed, started, or
reachable through the `Vmm` port at any method — per system constraint 9
and US-VM-9 it is a **sidecar `VmDriver` spawns and supervises directly**
(real `Command::spawn`, the same shape `ExecDriver` already uses for its
own workload process; no port trait sits between `VmDriver` and the OS for
it — see row 14 above). `vmm_override` therefore has nothing to substitute
for S-VM-67: injecting a `SimVmm` changes what `discover` / `probe` /
`create` / `terminate` return, and virtiofsd's `--sandbox=namespace`
capability check is not downstream of any of those four calls.

Two honest paths existed for S-VM-67 — doing either mints a new port trait
or narrows an already-accepted DISTILL scenario, both outside the two gaps
this amendment closed, which is why this ADR pair did not choose between
them at the time:

(a) Slice 04 mints its own storage-daemon supervision port when it is
    designed, carrying the same `probe` / fault-injection-table shape this
    section pins for `Vmm`, with its own `ServerConfig` override field
    following this same pattern; or

(b) the `--sandbox=namespace`-unavailable case is asserted at a narrower
    level than a real `overdrive serve` — a pure unit test over the
    spawn-argument construction plus a Tier-2-shaped fail-closed
    assertion — rather than through production `serve` + `deploy`.

This was recorded here so the gap was visible rather than silently assumed
solved by proximity to S-VM-13 / S-VM-51's (real) seam.

> **SUPERSEDED 2026-08-18 — volumes cut (Amendment 2026-08-18).** This entire S-VM-67 /
> `--sandbox=namespace` ruling is withdrawn: `virtiofsd` is not part of this feature at
> all. Its sandbox posture becomes GH #43's (virtiofsd lifecycle) / #258's (daemon
> posture) when that work is designed. The blockquote below is retained as history.
>
> **Ruled 2026-08-11, by user ruling — closes the open item above. Path
> (b).** The `--sandbox=namespace` posture is verified at the
> **launch-argument construction layer** — the same enforcement tier
> ADR-0082 § D2.1 already uses for `image_type=raw`: a value with private
> fields, exactly one rendering site able to produce the argument, and a
> pure unit test asserting on the rendered value. The spike's own evidence
> is why this shape is right, not merely convenient: `image_type=raw`'s
> *absence* surfaced **two layers away**, as a virtiofs `ConnectionRefused`
> (D2.1's own opening lie), so asserting at the argv-construction layer —
> where the value is produced — rather than on a downstream symptom is the
> pattern that already works here, twice.
>
> **The negative decision, stated plainly: this feature mints no
> storage-daemon supervision port.** Path (a) is explicitly not taken. If
> Slice 04's virtiofsd lifecycle later needs a supervision port on its own
> merits — process supervision, restart, health — that is a design decision
> made **then**, on those merits, never inherited from this ruling and never
> introduced as test scaffolding to reach S-VM-67 through a real
> `overdrive serve`.
>
> **What the argv-layer assertion covers, and what it honestly does not.**
> Rendering `--sandbox=namespace` at one site, with no second call site able
> to construct the argument and no field that could carry `chroot`, is
> **verifiable purely** — a unit test on the rendered value is a complete
> proof of what argument `VmDriver` constructs. It is **not** a proof that a
> *running* `virtiofsd` actually enforces that sandbox, nor that a host
> which genuinely cannot supply `--sandbox=namespace` is observed and turned
> into a `Failed` allocation rather than `virtiofsd` degrading — or failing
> to start — silently underneath a correctly-rendered argv. That second
> half is a **Tier-3 property of Slice 04**, exercised against a real
> supervised `virtiofsd` process when that slice is designed and built; it
> is not discharged by this ruling.
>
> **And, honestly, against `[D8d]`'s own requirement — fail-closed, never
> silently downgraded.** The silent-downgrade bug this feature exists to
> correct (the reference implementation's unrecorded `namespace` → `chroot`
> drift) is made **lint/test-detected by this ruling, not structurally
> unrepresentable at the type level** — the same tier D2.1 itself declares
> for `image_type=raw`: *"private fields + one rendering site + a lint
> clause… not a type-level impossibility."* A future edit to the one
> rendering function could still emit `chroot`; what this ruling buys is
> that there is exactly one place in the workspace such an edit could
> happen, and a pure unit test plus a lint clause catch it there. A stronger,
> type-level claim is Slice 04's to earn when it is designed, the same way
> D2.1 earned its own precision correction — by naming the enforcement
> mechanism exactly, never by asserting more than the type system delivers.

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

**Addendum, 2026-08-11 (D-3 fold-in).** A second, narrower read need arose —
`memory.events`' `oom_kill` counter for the VM exit-watcher's diagnosis — and
did **not** reopen this verdict. `CgroupFs`'s write-only contract is a
property of the trait, not scoped to `VmHostState`'s enumeration; a single
already-known cgroupfs path is still a read `CgroupFs` was never built to
expose. It landed on its own `CgroupAccounting` port rather than either
`CgroupFs` or `VmHostState` — see ADR-0082 § D8 for the full reasoning,
including why `VmHostState`'s cadence and crate boundary also didn't fit.

### A9 — A single `ReclaimAllocation { alloc_id, authors_ending: bool }`

**Rejected — DD-5 forbids it, and the reason is the same one DD-2 gives for
`ExitEvent.intentional_stop`.** One command authors an Ending Class and the other
must not; a boolean parameter puts that classification in the **caller's** hands,
which is a sentinel where a sum type belongs (§ *Type-driven design*). It is also
the one failure a test can miss: a collapsed implementation still kills the VMM
and still passes the *"terminal allocation's VMM is killed"* AC, and betrays
itself only against the **row-byte-unchanged** assertion (brief § 105a.10 AC 5).

### A10 — A `dataplane_override`-shaped `compose_vmm = vmm_override.is_none()` gate

*(Added 2026-08-11, gap-closure amendment, § D8.)* Mirror `dataplane_override`
exactly: an override field that, when set, skips `compose_production_driver`'s
Vm branch entirely and lets the test wire its own `VmDriver` by hand.

**Rejected.** This is the shape ADR-0074 (#248) found broken, reproduced on
purpose to show why § D8 does not copy it. Skipping composition rather than
substituting one port binding means the registry, the exit-observer loop,
`alloc_drivers` and `MtlsInterceptWorker`'s gate are **not** exercised by the
test — exactly the *"hand-installing a missing production effect"* shape
system constraint 1 forbids, and precisely what let #248's `AllocBackend`
discriminator ship dead on the one production path it needed to guard.
`vmm_override` (§ D8) is a narrower substitution: one port binding changes,
everything downstream stays real.

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
- **§ D8 closes the gap between what `brief.md` § 100 already asserted
  (*"`SimVmm` is the injection point"*) and how a crafter reaches it.**
  S-VM-13 and S-VM-51 are buildable against a pinned field and a pinned
  composition-root diff rather than an improvised seam, and the seam is
  provably no wider than `mtls_identity_override`'s already-shipped pattern
  — nothing downstream of the one port binding changes.

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
- **`Driver` gains two defaulted methods** (`live_allocations`, and —
  per D7 item 2a below — `release_supervision`), exercising intake I-2's licence
  that the first two drafts recorded as deliberately unexercised. Both are
  minimal and fail-safe (`None` ⇒ authorises nothing; the release default is a
  no-op), but they are changes to the trait every driver implements.
  *(Widened from one method 2026-08-11, iteration-2 review NEW-1.)*
> **SUPERSEDED 2026-08-18 — volumes cut (Amendment 2026-08-18).** The next two bullets
> (S-VM-67's absent seam, and `ExitEvent`'s second `storage_daemon_died` field) concern
> `virtiofsd` / volumes and no longer apply: there is no `virtiofsd` in this feature and
> `ExitEvent` retains only `oom` (row 13). Deferred to #97 / #43. Retained as history.

- **S-VM-67 has no seam this ADR pair supplies, and that is stated rather
  than papered over** (§ D8). `virtiofsd`'s `--sandbox=namespace`
  capability check sits outside the `Vmm` port entirely (no volume field
  reaches `VmConfig`), so `vmm_override` cannot inject its unavailability.
  **Ruled 2026-08-11, by user ruling:** the fail-closed posture is instead
  verified at the launch-argument construction layer — a pure unit test on
  the rendered `--sandbox=` value, the same enforcement tier as D2.1's
  `image_type=raw` — never through a real `overdrive serve`. **This
  feature mints no storage-daemon supervision port.** That assertion
  proves what argument `VmDriver` constructs; it does not prove a running
  `virtiofsd` enforces it or fails closed when the host cannot — that
  remains a Tier-3 property of Slice 04. § D8's closing section carries the
  full statement.
- **`ExitEvent` gains a second additive field** (`storage_daemon_died`,
  row 14) beside `oom` (ADR-0082 § D8), and `exit_observer::handle_exit_event`
  gains a second precedence check — this one checked **ahead of** `ExitKind`
  rather than nested inside a `Crashed` arm, which is a different shape
  from row 13's and must not be copy-pasted from it verbatim.

### Neutral

- `DriverType::MicroVm` is deleted and `DriverType::Vm` survives (intake I-5).
  Both variants already exist (`traits/driver.rs:43`, `:45`); the deletion costs
  two exhaustive-match arms, an OpenAPI regeneration (`DriverType` derives
  `ToSchema` transitively through `TransitionSource`), and an amendment to the
  now-false *"existing variants never change their wire form"* docstring at
  `traits/driver.rs:26-29`. `DriverType` reaches **no** persisted row, so there
  is no rkyv envelope bump for it.
- Adding `WorkloadDriver::Vm` **is** an rkyv event — `WorkloadIntentEnvelope`
  V1 → V2 via the full six-step single-commit procedure, user-ruled at intake
  I-5. The `FIXTURE_V1_*` golden bytes are **regenerated in the same commit as
  the bump** (step 01-02, `8507f631`) — the sanctioned same-commit exception to
  the "never touch `FIXTURE_V1`" rule, forced by rkyv 0.8's max-variant
  enum-root sizing (see the Amendment's **Test mechanics** below for the
  mechanism). **The exact payload-type fork is pinned in the Amendment
  2026-08-12 section below** (`JobEnvelope` was ADR-0050-deleted; the live
  envelope is `WorkloadIntentEnvelope`).

---

## Amendment 2026-08-12 (DELIVER 01-02 — `WorkloadIntentEnvelope` V1→V2 fork, GH #42)

**Context.** The Consequences "Neutral" note above records that adding
`WorkloadDriver::Vm` is an rkyv schema event, and the earlier text named the
envelope "`JobEnvelope` V1 → V2". That name was stale: `JobEnvelope` was
**deleted by ADR-0050**; the live envelope is `WorkloadIntentEnvelope`. This
amendment corrects the name and pins the exact payload-type fork DELIVER 01-02
lands, per ADR-0048 (alias-to-payload + the six-step version-bump procedure).

**Why the archived layout shifts.** `WorkloadDriver` is embedded by the frozen
V1 payloads — `JobV1`, `ServiceV1` and `ScheduleV1` each carry
`driver: WorkloadDriver`. Growing that enum with a `Vm` variant shifts the
archived layout of all three, so their `FIXTURE_V1_*` golden bytes would break
without a version bump. The fork is uniform across all three inner payloads.

**FROZEN** (embedded by the frozen V1 payloads; byte-identical to today's
Exec-only `WorkloadDriver`):

- `WorkloadDriverV1 { Exec(Exec) }`
- `JobV1` / `ServiceV1` / `ScheduleV1` re-point `driver:` to
  `WorkloadDriverV1` — the **inner** V1 payload types are byte-identical to
  today's Exec-only shapes (a single-variant enum's own layout is unchanged).
  The **outer** `WorkloadIntentEnvelope` archived root, however, grows with the
  V2 bump (rkyv 0.8 sizes an enum's root to its largest variant), so the
  `FIXTURE_V1_*` golden bytes do NOT survive untouched — they are regenerated
  at the V2-inclusive layout in the same commit (see **Test mechanics** below).

**LIVE / V2:**

- `Vm { command: String, args: Vec<String>, kernel: String, rootfs: String }`
  — mirrors the runtime `VmPayload` (§ D3) **minus `volumes`**; `String` not
  `PathBuf` (rkyv/serde-clean, matches `Exec.command`); volumes deferred to
  Slice 04; same derive set as `Exec`.
- `WorkloadDriverV2 { Exec(Exec), Vm(Vm) }` ; `pub type WorkloadDriver = WorkloadDriverV2`
- `JobV2` / `ServiceV2` / `ScheduleV2` = copies of the V1 shapes with
  `driver: WorkloadDriverV2` ; `pub type Job = JobV2` ;
  `pub type Service = ServiceV2` ; `pub type Schedule = ScheduleV2`
- `WorkloadIntentV2 { Job(JobV2), Service(ServiceV2), Schedule(ScheduleV2) }` ;
  `pub type WorkloadIntent = WorkloadIntentV2`
- `WorkloadIntentEnvelope::V2(WorkloadIntentV2)` appended ;
  `Latest = WorkloadIntentV2` ; `latest() -> V2`
- `From<WorkloadIntentV1> for WorkloadIntentV2` — structural
  Job→Job / Service→Service / Schedule→Schedule, driver
  `WorkloadDriverV1::Exec → WorkloadDriverV2::Exec`
- `into_latest()` extended to chain V1 → V2

**Uniform-fork rationale.** § D4 rejects `[vm]` + `[service]` (a Service can
never be a Vm) but accepts `[vm]` + `[job]` and `[vm]` + `[schedule]`. All
three inner payloads still fork to V2 (Service embeds `WorkloadDriverV2` too)
because (a) it preserves today's uniform embedding, (b) "Service-with-Vm" is
prevented at the **parse gate** (§ D4) — it need not be made unrepresentable at
the type level, and (c) leaving `Service` on `WorkloadDriverV1` creates a
V1-vs-V2 type mismatch at live `Service`-construction sites.

**Test mechanics** (`workload_intent.rs`). rkyv 0.8 sizes an archived enum's
root to its **largest** variant, so appending the larger
`WorkloadDriverV2 { Exec, Vm }` grew the `WorkloadIntentEnvelope` archived
root. The pre-V2 `FIXTURE_V1_*` golden bytes, serialized at the V1-only root
size, therefore no longer round-trip through the grown envelope
(`rkyv::from_bytes::<WorkloadIntentEnvelope>` on them fails) — so they were
**regenerated** as an explicit `WorkloadIntentEnvelope::V1(…)` encoding at the
V2-inclusive layout, in the **same commit as the bump** (step 01-02,
`8507f631`). This is the **sanctioned same-commit-regeneration exception** to
`.claude/rules/testing.md`'s "never touch `FIXTURE_V1`" rule: that rule exists
to stop *silent* drift, not a deliberate regeneration landed alongside the
version bump that forces it. The regenerated fixture is built from the explicit
frozen V1 types (**not** the live aliases, which now encode V2); `into_latest()`
then converts the decoded V1 → V2 for the comparison against the `Latest`(V2)
projection. The `discriminant_offset_from_end() == None` deferral
(`workload_intent.rs:37-43`) is **unaffected** by the V2 bump — the V1 roundtrip
+ `archive_for_store` roundtrip remain the defense.

**Destructures.** Every irrefutable `let WorkloadDriver::Exec(..) =` becomes an
exhaustive `match` with a `Vm` arm. Where the `Vm` behaviour needs step 01-08's
`AllocationSpec.driver` / `DriverPayload` work, the arm is
`todo!("RED scaffold: ... 01-08")` gated with `#[expect(clippy::todo)]`.

---

## Amendment 2026-08-16 — typed start-failure transport and Phase-03 recovery

Checkpoint `3222f030` demonstrated that the old D5 shape was internally
contradictory: it claimed VM classification came from typed `VmmError`, while
requiring `VmDriver` to flatten that value into `StartRejected.reason: String`
and the action shim to classify the string later. It also grouped the boot
clock, `VmmExit`, VM-local storage, and mid-run exit facts under `VmmError`,
although those facts belong to different owners.

The revised D5 is the single current contract. It makes source ownership
explicit, preserves the complete Exec surface, appends the known post-READY
command-dispatch cause, and keeps rows 13/14 outside the start-error type. The
roadmap recovery is recorded in feature DWD-24: completed step 03-01 remains a
checkpoint, then a typed upstream vertical slice and the remaining named
vocabulary discharge S-VM-33…41 without changing execution history.

Rejected alternatives are the same three ruled in ADR-0032's 2026-08-16
amendment: direct `TransitionReason` carriage (too broad), per-driver text
parsers (lossy and presentation-coupled), and independent driver/cause fields
(contradictory states representable). No compatibility parser remains.

---

## Amendment 2026-08-17 — § D3's `VmPayload.kernel` / `.rootfs` become load-bearing; the node-level artifact seam is deleted; VM composition is unconditional

**Scope: application architecture only. No § D3 field is added, removed or
retyped; no new operator surface is created. This amendment makes the ADR's
already-accepted per-allocation artifact fields actually reach the driver, and
deletes the test-only node-level seam that stood in for them.**

### The gap this closes, grounded rather than asserted

§ D3 has always declared `VmPayload.kernel: PathBuf` and
`VmPayload.rootfs: PathBuf` — both annotated `// operator surface, BYO
artifact` — and § D4's `[vm]` block has always carried `kernel` and `rootfs`
keys. The 2026-08-12 amendment above persisted both into the `V2` envelope.
Every hop of that path is live today and was verified end to end:

- `[vm]` TOML → `workload_spec.rs` `VmInput { command, args, kernel, rootfs }`
  (`#[serde(deny_unknown_fields)]`);
- → wire DTO `aggregate::VmInput` → `JobV2::from_submit` →
  persisted `aggregate::Vm { command, args, kernel, rootfs }` in the rkyv `V2`
  envelope;
- → `WorkloadLifecycle` projects both arms
  (`StartAllocation` and `RestartAllocation`) into
  `DriverPayload::Vm(VmPayload { kernel: PathBuf::from(kernel),
  rootfs: PathBuf::from(rootfs), .. })`;
- → the action shim passes `spec` through unmodified and calls
  `driver.start(&spec)`.

**The values then die at the consumer.** `VmDriver::provision_vmm` reads
`self.layout.kernel` and `self.layout.rootfs_master` — a node-wide
`VmHostLayout` built once at boot — and never pattern-matches `spec.driver`
into its `Vm` arm at all (`spec.driver` is touched only via the
kind-agnostic `.command()` / `.args()` accessors). Two workloads on one node
therefore boot the same image regardless of what either spec says. The
operator surface this ADR ratified is, today, decorative.

That node-wide layout is fed by `ServerConfig.vm_artifacts:
Option<VmBootArtifacts>`, which is `#[cfg(feature = "integration-tests")]`,
as are the composition block that consumes it and the two `serve` entrypoints
that set it. So `vm_artifacts = Some(_)` is **a state only a test seam can
produce** — precisely the shape CLAUDE.md § "Ground the premise" forbids
building on. In a shipped binary no `Vm` driver is ever composed, and every
`[vm]` deploy lands `Failed` with `no vm driver composed on this node`
(executed evidence: `verification/expectations/E06-vm-job-deploy-reaches-running/`,
SHA `655ac964`, on a real x86_64 + KVM host).

### D3a — The artifact contract is per-allocation. There is no node-level artifact configuration.

> **Amended 2026-08-18 (ADR-0082 fourth amendment — the confined-artifact-access
> mechanism relocates the per-launch clone; this is the companion § D3a follow-up
> the fourth amendment surfaced for ratification, now ratified, implemented and
> reviewed in DELIVER phase 04).** The prose below — and in §§ D3b, D3f, D3i and
> the § D7 correction — originally said the per-launch rootfs clone lands **beside
> the operator's own `[vm] rootfs`, on the master's own filesystem, wherever the
> operator placed it**. That is now stale. Per
> [ADR-0082](adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md)'s
> **fourth amendment (§ (c-fix.2), the governing decision)**, the FICLONE clone
> lands in a **platform-owned staging root** —
> `overdrive_core::vm::config::clone_staging_dir(data_dir)`, a sibling of
> `clone_index_dir(data_dir)`, threaded through `VmHostLayout` as a new
> `clone_staging_dir: PathBuf` field — and **never** in the operator's directory
> (reaching a clone under the operator's dir would require `o+x` traverse on an
> operator-owned path, the DAC regression B1 the amendment reverses). Two
> consequences carry into § D3a:
>
> 1. **The rootfs master must reside on the VM data filesystem** — the same
>    filesystem as `clone_staging_dir(data_dir)` — because FICLONE is an
>    intra-filesystem ioctl (C-1, no full-copy fallback) and must stage the clone
>    on the master's own filesystem. On the appliance the one durable data
>    partition holds BYO artifacts by construction, so the constraint only
>    excludes a master an operator placed on a separate mount. The **kernel**
>    carries no such constraint — it is COPIED (not cloned) into the per-alloc run
>    dir, so it may live on any host path.
> 2. **A foreign-filesystem master fails closed.** `Vmm::create` observes the
>    cross-device FICLONE as `EXDEV` and returns
>    `VmmError::ConfinementUnavailable { control: UidDrop, .. }` (→
>    `TransitionReason::VmConfinementUnavailable { control: UidDrop }`) — never a
>    silent operator-dir widening, never a C-1-defeating full-copy fallback.
>
> **The clone-index reclamation semantics of §§ D3f–D3h are UNCHANGED by this
> relocation.** `RootfsPlan::clone_dest` moves only its *parent directory* (from
> `parent(master)` to the platform staging root) while KEEPING its existing
> `.overdrive-vm-rootfs-<alloc>.img` filename; the `index_link` symlink still
> points at `clone_dest` **wherever it lives**, so `observe_clones`,
> `discard_artifacts`, the create-before / remove-after ordering, and
> `RealVmHostState`'s alloc-from-filename attribution all behave exactly as
> §§ D3f–D3h document. Only the clone's parent directory moved — the
> record-don't-re-derive design (§ D3f) is precisely what makes that move
> invisible to reclamation; it is the very "future move of `clone_dest`" that
> design was built to tolerate (§ Consequences, *"A future move of `clone_dest`
> cannot silently blind the sweep"*). `RootfsPlan::for_alloc` gains the staging
> root (`for_alloc(master, master_bytes, alloc, staging_dir, index_dir)`); no
> `Vmm` method, no reconciler `State` / `View`, and no `Action` variant changes.

`VmDriver::start` resolves the kernel and rootfs for an allocation **from that
allocation's own `VmPayload`**, never from node state:

```rust
// in VmDriver::provision_vmm, before any provisioning
let DriverPayload::Vm(payload) = &spec.driver else {
    return Err(start_rejected_unclassified(format!(
        "VmDriver received a {} payload", spec.driver.driver_type()
    )));
};
// payload.kernel : &Path   — the [vm] spec's kernel
// payload.rootfs : &Path   — the [vm] spec's rootfs
```

No accessor is added to `DriverPayload`. `VmPayload`'s fields are already
`pub`; the `let …else` refutable binding reaches them and states the routing
precondition in the same breath. A non-`Vm` payload reaching `VmDriver` is a
registry-routing defect, so it takes the existing
`DriverStartClass::Unclassified { driver: DriverType::Vm }` fallback — no new
class, no new variant.

**One private signature changes, and it is pinned here so it is not
improvised.** `VmConfig.kernel` is a `KernelImage`, whose only constructor is
`KernelImage::validate(path, arch, header)` over a private field, while the
payload supplies a bare `PathBuf`. `preflight_kernel` therefore becomes:

```rust
async fn preflight_kernel(path: &Path, arch: HostArch)
    -> Result<KernelImage, DriverError>
```

— it stops discarding the validated value (`.map(|_| ())` today) and returns
it for `VmConfig` to consume. **No new `KernelImage` constructor is added**:
no `from_path`, no `new_unchecked`, no relaxation of the private field.
`KernelImage::validate` stays the sole constructor and is itself unchanged.
Its three failure arms (`KernelNotFound`, the unclassified read error,
`KernelFormatUnsupported`) are unchanged. This is the one place a crafter
would otherwise be tempted to invent surface on a `core` type — CLAUDE.md
§ "Implement to the design — never invent API surface" governs, and the
signature above is the whole permitted change.

Consequently `VmHostLayout` sheds **exactly two fields**, `kernel:
KernelImage` and `rootfs_master: PathBuf`. What remains — `cgroup_root`,
`run_dir_root`, `arch`, `vcpus`, `confinement` — is genuinely node-invariant
and stays. `VmHostLayout`'s own doc comment ("Slice 01 ships a single fixed
template per node — no per-allocation BYO kernel/rootfs surface exists yet")
becomes false on this change and is corrected in the same commit.

`VmBootArtifacts`, `ServerConfig.vm_artifacts`, and the `serve` entrypoints
`run_with_dataplane_and_vm_artifacts` / `run_with_vm_artifacts` are
**deleted**, not ungated. Nothing replaces them: with artifacts arriving per
allocation there is no node-level artifact to configure, so the seam has no
production counterpart to be promoted into. Their test callers move the
artifact paths into the `[vm]` spec they deploy — which is what a real
operator does, so those tests become *more* production-faithful, not less.
`vmm_override` and `run_with_dataplane_and_vmm_override` **stay** gated: § D8's
adapter-substitution fault-injection seam is a genuine test-only capability
with no production analogue, and is untouched here.

### D3b — Where the kernel is validated, and why that is still Earned Trust

`KernelImage::validate` is a pure validator over a bounded magic window; the
imperative shell does the read. Under D3a its only call site is
`preflight_kernel`, which already runs **per allocation, immediately before
`Vmm::create`**, and already re-reads the path from disk. Its own doc comment
already describes itself as "Per-allocation kernel preflight". So validation
is not weakened, relocated or deferred — the *redundant boot-time copy over a
node-wide path* is what disappears, along with the node-wide path itself.

The Earned-Trust posture is preserved by putting each proof where its subject
lives:

| Subject | Proven | Where |
|---|---|---|
| The **host can run microVMs** (`cloud-hypervisor` binary, `/dev/kvm`, run dir, reflink on the node's own `image_dir`) | once, at boot | `Vmm::probe` — unchanged |
| **This allocation's kernel** is present and loadable for this arch | at every start | `preflight_kernel` → `KernelImage::validate` |
| **This allocation's rootfs** is present and stat-able | at every start | the existing rootfs preflight |
| **This allocation's rootfs directory supports `FICLONE`** | at every start, by the clone itself | `Vmm::create` → `ficlone_rootfs` |

"Prove it once, use it many times" was always a statement about the
*hypervisor capability*, which is node-scoped. A per-workload artifact has no
"many times" to amortise over — it is proven for the allocation that names it,
which is the only honest scope. `preflight_kernel` keeps returning
`VmStartFailure::KernelNotFound` / `KernelFormatUnsupported`, and the rootfs
preflight keeps returning `RootfsNotFound`, all naming the path the operator
actually wrote.

**The reflink probe's scope narrows, and that is stated rather than glossed.**
`Vmm::probe` runs `probe_reflink` against the adapter's own `image_dir`
(`/srv/vm` by default), but `RootfsPlan::for_alloc` derives the clone
destination from the *master's own parent directory* — which, under D3a, is
wherever the operator's `[vm] rootfs` lives. `FICLONE` is intra-filesystem, so
a boot probe on `/srv/vm` **no longer proves** the per-launch clone will
succeed for a rootfs staged on a different filesystem, or on one without
reflink support at all. The boot probe keeps its value (it is still the
node's own staging area, and still catches the node-level misconfiguration it
was written for); it simply stops being a proof about operator-chosen paths.
This is the honest residual of making artifacts per-allocation, and it is a
real narrowing of what boot-time Earned Trust buys.

> **Corrected by the ADR-0082 fourth amendment (2026-08-18); see § D3a's
> amendment note.** This paragraph's location premise — "the clone destination is
> the master's own parent directory, wherever the operator's `[vm] rootfs` lives,
> possibly on a different filesystem" — is stale. Under the fourth amendment
> (§ (c-fix.2)) `RootfsPlan` stages the clone in the platform-owned
> `clone_staging_dir(data_dir)` on the master's filesystem, and the master **must**
> reside on the VM data filesystem: a foreign-filesystem master fails closed as
> `ConfinementUnavailable { control: UidDrop }` (FICLONE `EXDEV`) rather than
> staging anywhere. The *narrowing* this paragraph names still holds in substance
> — a boot `probe_reflink` on `image_dir` is not itself a proof about the
> clone-staging filesystem — but its *location* is now the platform staging root,
> not an operator-chosen directory; S-VM-94 / step 03-08's sixth criterion
> continue to own the same-filesystem non-reflink fail-closed, with the EXDEV
> foreign-filesystem fail-closed layered on top.

Two consequences follow, and neither is deferred silently:

- **The fail-closed behaviour is already scenario-owned.** S-VM-94 ("the
  per-launch `FICLONE` clone fails closed on a non-reflink target — self
  application of the boot probe's own rule") is the existing ratified
  scenario for exactly this. Under D3a its target becomes the
  **operator-named** rootfs directory rather than the node default; the
  scenario is unchanged in substance and does not move.
- **Today that failure is unclassified.** `ficlone_rootfs` surfaces an ioctl
  failure as `VmmError::Io`, which `VmDriver` maps to
  `DriverStartClass::Unclassified` — i.e. it renders as an internal-shaped
  error, the very shape D3d exists to remove. This amendment does **not**
  mint a `VmStartFailure` variant for it: doing so would be new API surface
  on a `core` type, S-VM-94 already owns the behaviour, and the correct
  moment to type it is when that scenario is implemented. **It is recorded
  here as a known, named residual, not as a solved problem** — see
  Consequences below.

Note for whoever implements S-VM-94: **E06 cannot catch this.** E06 stages its
artifacts under `/srv/vm/overdrive-testing`, i.e. beneath the probe's own
default, so a passing E06 says nothing about a cross-filesystem rootfs. The
instrument that measures K4 is not an instrument for this residual.

### D3c — VM composition is unconditional and gated only by `Vmm::probe`

§ D2's "the registry **is** the VM capability gate" is unchanged in intent and
finally true in production. `compose_vm_driver` loses its `&VmBootArtifacts`
parameter and its `#[cfg]`; `run_server`'s call site loses both the `#[cfg]`
and the `if let Some(artifacts) = config.vm_artifacts` guard. The composition
sequence becomes exactly discover → probe → insert, with the same three
outcomes as today:

- probe passes → `registry.insert(VmDriver)`;
- probe fails with no override injected → `VmComposeError::NotAvailable` →
  `tracing::info!(name: "driver.vm.not_composed", reason = %cause, …)`, the
  node boots, no `Vm` entry. **Capability absence remains a first-class
  answer, not a fault** — a node without `cloud-hypervisor` still starts;
- probe fails with a § D8 `vmm_override` injected → `VmComposeError::Refused`
  → `health.startup.refused` + `ControlPlaneError::VmmBoot`, boot refused.

The hardcoded `vcpus: 1` and the `VmConfinement` uid/gid remain exactly as they
are; § D3a changes artifact supply only, and re-pointing those at real
production values is Phase 04/06 scope, not this amendment's.

### D3d — Capability absence must name a capability, not an internal error

With D3c live, a node has no `Vm` registry entry for exactly one reason:
`Vmm::probe` did not pass. The dispatch-time registry-miss fallback keeps its
`DriverStartClass::Unclassified { driver }` class — the action shim cannot and
must not classify per-driver causes (it "owns persistence only") — but its
`detail` must name the capability and point at the executed boot reason rather
than reading as an internal defect. Per DWD-24, `detail` is free-form verbatim
diagnostic text and is **never** a classification input, so this changes no
contract and no conversion.

**No new `TransitionReason` variant is minted, and none is needed.** Three
reasons, stated so the next reader does not re-litigate it:

1. The registry miss is **driver-kind-generic** — it fires identically for a
   future `Wasm` deploy on a node without `wasmtime` — so a `Vm`-prefixed
   variant would be the wrong shape, and a generic one duplicates
   `DriverInternalError`'s slot.
2. Reusing `VmStartFailure::HypervisorAbsent { searched }` would require the
   action shim to synthesise driver-specific knowledge (the searched paths) it
   does not have — the exact per-driver branching § D5's delivery notes forbid
   there.
3. The **properly typed** answer is the admission-time rejection § D2 already
   designs ("`[vm]` deploys are rejected at admission naming the absent
   capability"). Feature DWD-23 ratified the dispatch-time fallback as a safe
   interim and is the record of that decision. **This amendment does not
   build the admission-time gate and makes no promise about when it is
   built** — it is simply not in scope here, and no forward pointer is
   written in its place. (DWD-23 recorded "no GitHub issue created — follow-up
   surfaced for approval"; that remains true and unchanged. Per CLAUDE.md
   § "Deferrals require GitHub issues", no number is invented here and none
   is implied.)

### D3e — Supersession: this operator surface is deleted by the image factory

`[vm] kernel` / `[vm] rootfs` are a **slicing mechanism, not a product
commitment** (feature-delta.md changelog, user ruling 2026-08-11). They exist
so the driver ships without blocking on the image factory, GH
[#259](https://github.com/overdrive-sh/overdrive/issues/259) (OCI / Dockerfile
→ bootable rootfs image factory), whose acceptance requires `overdrive deploy`
to accept an OCI reference with no operator-side rootfs preparation.

When #259 lands it **deletes the two TOML keys** and replaces them with an
image reference. What survives is everything below the spec: the factory
resolves the reference into host paths and fills the same
`VmPayload.kernel` / `.rootfs` fields, and `VmDriver` is unchanged. That is a
single cut at one boundary, and it is only available because artifacts are
per-allocation — a node-level template could not survive #259 at all, since
two workloads on one node resolve to two different images.

### Rejected alternative — node-level artifact configuration on `serve`

Adding `--vm-kernel` / `--vm-rootfs` flags (or config-file keys, or env vars)
to `overdrive serve` and ungating `vm_artifacts` was considered and rejected:

- **It contradicts this ADR.** § D3 already ratifies per-allocation
  `kernel`/`rootfs`. A node-level flag would leave those fields decorative and
  make the platform silently ignore what the operator wrote in the spec — an
  operator writing `kernel = "/a"` would get `/b`, with no diagnostic.
- **It is larger, not smaller.** It adds a clap surface, a `ServeArgs` field,
  main.rs plumbing and a validation path, and *keeps* `VmBootArtifacts`,
  `ServerConfig.vm_artifacts` and the two gated entrypoints alive. D3a adds no
  operator surface and deletes four items.
- **It does not survive #259.** #259 resolves per-workload images; a node-wide
  template would have to be deleted wholesale and the per-allocation path
  built anyway — two cuts instead of one.
- **It cannot be measured by the instrument that exists.** E06's runner already
  deploys a spec whose `[vm]` block names `kernel` and `rootfs`. Under D3a it
  re-runs unchanged. Under the flag design the expectation's own runner would
  have to be rewritten to match the implementation — self-assessment of the
  kind `.claude/rules/verification.md` § Enforcement rejects.

### Consequences

- **Positive.** The feature becomes production-drivable through `overdrive
  serve` + `overdrive deploy` with no test-only wiring, which is KPI **K4**'s
  binary bar. Per-workload images become possible, which every later slice and
  #259 require. Four test-only items leave the production type surface. The
  spec fields this ADR ratified stop being decorative.
- **Negative.** A malformed or absent artifact is now discovered at allocation
  start rather than at `serve` boot, so a node can start healthy and still
  reject an individual `[vm]` deploy. That is the correct scope for a
  per-workload input, and the failure vocabulary for it (`VmKernelNotFound`,
  `VmRootfsNotFound`, `VmKernelFormatUnsupported`) already exists and already
  names the exact path.
- **Negative.** Deleting `VmBootArtifacts` breaks **every call site that names
  it**, not an enumerated subset. That is roughly thirty construction sites
  across `vm_walking_skeleton.rs`, `vm_boot_failure_vocabulary.rs` and
  `vm_reclamation_tier3.rs`, the `spawn_vm_server`-shaped helpers they share,
  **and** one struct-literal outside the VM test files entirely —
  `overdrive-control-plane/tests/integration/workload_lifecycle/
  convergence_loop_spawned_in_production_boot.rs`, which carries
  `vm_artifacts: None`. All of them are compile breaks, all are in scope for
  the step that lands D3a, and the scope list must be read as "every call
  site" rather than a scenario enumeration.
- **Negative.** Steps 03-05 and 03-06 landed S-VM-33/34/35/36/41 against "the
  path `serve` composed against". Those scenarios are unchanged in substance —
  a missing kernel, a missing rootfs or a wrong-format kernel is still named
  precisely — but their fixtures must now mutate the path the *spec* names,
  and the shared `VmBootArtifacts`-taking helper they call disappears. The
  step that lands D3a owns that update; it is a fixture relocation, not a
  scenario change, and **no S-VM ID moves**.
- **Negative, and genuinely unresolved.** Per D3b, `Vmm::probe`'s reflink
  proof no longer covers the operator's rootfs directory, and a `FICLONE`
  failure there currently renders as an *unclassified* internal-shaped error.
  S-VM-94 owns the fail-closed behaviour and this amendment types nothing for
  it. Recorded as a known residual, not solved.
- **Negative.** The per-launch rootfs clone is now written into an
  **operator-chosen** directory (the parent of `[vm] rootfs`) rather than a
  platform-owned one. That directory may be read-only, shared between
  workloads, or on a filesystem the platform does not control. No new
  operator surface exists to redirect it; this is a direct consequence of
  deriving the clone destination from the master's own parent.
- **Negative.** Phases 04, 05 and 06 have not executed and both edit
  `vm_driver.rs`, which D3a restructures. Their leading steps must take a
  dependency on the step that lands D3a so the ordering is encoded in the
  graph rather than asserted in prose.
- **Neutral.** The `[vm]` grammar, the `V2` envelope, `DriverPayload`,
  `AllocationSpec`, the typed `DriverStartFailure` contract and every
  `TransitionReason` variant are untouched.

Recorded in feature DWD-25. Delivered by roadmap steps 03-07 and 03-08.

## Amendment 2026-08-17 (second) — the per-launch rootfs clone becomes reclaimable; § D7's clone surface is corrected; `run_with_kek` is ratified

**Scope: application architecture only. No operator surface is created. No
`VmHostState` method signature changes, no `Action` variant is added, no
reconciler changes. This amendment closes a durable resource leak the first
2026-08-17 amendment opened, and corrects two § D7 statements that amendment
falsified without noticing.**

### The leak, traced rather than asserted

§ D3a moved the per-launch rootfs clone from a node-wide staging area onto the
parent directory of the operator's own `[vm] rootfs`. Its Consequences record
one half of what followed — *"the per-launch rootfs clone is now written into
an **operator-chosen** directory … That directory may be read-only, shared
between workloads, or on a filesystem the platform does not control"* — and
stop there. The half it does not record is the one that matters more: **nothing
reclaims that clone.**

Four facts, each read from the tree rather than inferred:

1. **`VmDriver::stop` still reclaims correctly, and by a route the operator
   directory cannot obstruct.** It extracts the per-allocation `RootfsPlan`
   from its own in-memory `live: Arc<Mutex<BTreeMap<AllocationId,
   VmSupervision>>>` and calls `tokio::fs::remove_file(rootfs.clone_dest())`
   on the exact path it minted at `start`. `cleanup_after_start_failure` does
   the same. Neither enumerates a directory, so neither cares where the clone
   landed. *(The finding's premise, verified: it holds.)*
2. **No other path in the driver removes it.** `run_exit_watcher` classifies
   the exit, transitions `Live → EndingInFlight` and emits an `ExitEvent`; its
   own doc comment states teardown "happens later, in `stop` or
   `cleanup_after_start_failure`, both driven by a SEPARATE caller." So an
   allocation that ends **without** `stop` — a natural exit, a guest crash, or
   a control-plane restart that loses the in-memory plan — leaves
   `<operator_rootfs_dir>/.overdrive-vm-rootfs-<alloc>.img` behind.
3. **The surface that exists to catch exactly that is pointed somewhere
   else.** `VmHostObservation.clones: BTreeMap<AllocationId, PathBuf>` is
   already the right shape, and `plan_reclamation` already unions `clones`
   into its host-id set and already treats a clone as a VM-exclusive trigger.
   But `RealVmHostState` enumerates clones from **one** directory, its
   `staging_dir`, and § D3a's composition hardcodes that to
   `/run/overdrive/vm-rootfs-staging`. `discard_artifacts` re-derives the same
   path independently. Neither is where `RootfsPlan::for_alloc` puts anything.
4. **And that directory is `tmpfs`, which makes the mismatch worse than a
   mismatch.** ADR-0082 § D2 gap 3's own pinned doc comment says the clone
   destination must be "derived on the **master's own filesystem** (FICLONE is
   intra-filesystem; **staging into `/run` fails `EXDEV`**)". So the one
   directory the platform watches for clones is a directory in which a clone
   *cannot be created*. The two statements are flatly contradictory and sit in
   the same tree.

**The sharper framing, and it is correct.** The clone surface only ever agreed
with the driver because the deleted `VmBootArtifacts` seam fed *both* sides
from the same node-level artifact path — `staging_dir` was derived from the
node's configured rootfs artifact's parent directory, which is precisely where
`RootfsPlan::for_alloc` staged. § D3a deleted the derivation on one side and
hardcoded a literal on the other. In production the divergence was invisible
because, as DWD-25 measured, **no VM had ever booted at all** (`K4` NOT MET,
`new_hypervisors=0`): all three of § D7's surfaces were vacuously empty, and
the clone surface specifically has never been exercised against a real
operator-chosen directory. § D7's three-surface reclamation model has been
**partly notional in production for its entire life.**

This is a durable, unbounded leak on a shared host — one reflink clone per
crashed launch, each growing toward the full rootfs size as the guest diverges
from its master, on a filesystem the operator relies on. It is exactly the
class SD-1 and § D7 exist to prevent.

### D3f — The clone location is authored once and *recorded*, never re-derived

**The root cause is not "the wrong directory constant." It is that the clone's
location is authored in `overdrive-core` (`RootfsPlan::for_alloc`) and
independently re-derived in `overdrive-host` (`RealVmHostState`), with nothing
binding the two.** The *filename* half of that agreement survived § D3a by
luck — both sides spell `.overdrive-vm-rootfs-<alloc>.img`, and the filename is
what carries attribution. The *directory* half diverged silently the moment one
author moved. Any fix that leaves two independent derivations in place merely
re-synchronises the bug class; per CLAUDE.md § "Type-driven design" and this
methodology's *make the bug class non-representable* rule, the derivation must
be eliminated, not re-aligned.

It is also **not recomputable**, which is the fact that selects the design.
`clone_dest` is a function of `parent([vm] rootfs)` and `AllocationId`, and the
first of those is **mutable and deletable by the operator**: editing a spec's
`rootfs` path, or deleting the workload, destroys the only input from which the
old clone's directory could be re-derived — while the clone itself remains on
disk. `development.md` § "A convergent record cannot answer 'did it happen'"
governs: *"some things are not recomputable, because convergence already
destroyed the input. Before deciding a value can be re-derived on read, check
that the state it would be derived from still exists."* Here it does not. The
location must therefore be **recorded at the moment the platform chooses it**.

**The record is a symlink, in a platform-owned index directory:**

```
<index_dir>/.overdrive-vm-rootfs-<alloc>.img  ->  <clone_dest>
```

- The **clone's parent directory moves once — and the index tolerates it by
  design.** As authored (2026-08-17) the clone stayed beside the master; the
  ADR-0082 fourth amendment (2026-08-18, § (c-fix.2) — see § D3a's amendment note)
  relocates its parent to the platform-owned `clone_staging_dir(data_dir)` on the
  master's (VM data) filesystem, so FICLONE stays intra-filesystem and § D3b's
  intra-filesystem premise is preserved (the master must reside on the VM data
  filesystem; a foreign-filesystem master fails closed). This relocation is
  exactly the "future move of `clone_dest`" this section is built to absorb:
  `index_link` records `clone_dest` **wherever it lives** rather than re-deriving
  it, so the filename-carries-attribution rule, the ordering contract, and the
  crash table below are **unchanged** — only the parent directory differs.
- The **filename is the existing one**, so `RealVmHostState`'s `CLONE_PREFIX` /
  `CLONE_SUFFIX` parsing and its `AllocationId::from_str` recovery are
  unchanged. Attribution still rides on the filename, exactly as ADR-0082 § D2
  gap 3 designed.
- The link is a **symlink and not a recorded path string** because `read_link`
  hands `observe_clones` the `PathBuf` its map already wants, needs no encoding
  decision, mints no serialisable type, and therefore touches neither the rkyv
  envelope discipline nor the CBOR `View` discipline.

**The ordering is the contract, and it is the reason this is complete:**

> **The link is created before the clone, and removed after the clone.**
> Therefore at every instant, a clone that exists has a link that exists.

Contrapositive: *no link ⇒ no clone*. Enumerating links enumerates a **superset**
of live clones, so the sweep cannot miss one. The two crash windows both
converge rather than leak:

| Crash point | Residue | Resolution |
|---|---|---|
| after link, before clone | dangling link, no clone | `observe_clones` reports it; `plan_reclamation` sees a VM-exclusive surface entry; `DiscardStrandedArtifacts` removes the (absent) target and the link — both `NotFound`-tolerant |
| after clone removal, before link removal (in `stop`) | dangling link, no clone | same sweep, same disposal |
| after clone, before link | **unreachable by construction** | this is the ordering the rule forbids |

This is the same *record-the-effect-before-attempting-it* discipline the
reconciler runtime already uses at STEP 7 → STEP 8 (fsync the `View`, *then*
dispatch the actions). It is explicitly **not** the marker anti-pattern
`.claude/rules/reconcilers.md` condemns: that marker was consulted **as the
diff**, standing in for observing the resource. This link *is* the observation
path — the sweep still reaches the real file through it, disposal is idempotent
whether or not the target exists, and no convergence decision is derived from
the link's mere presence.

### D3g — One derivation, two composition sites

`index_dir` is a **pure path derivation over the node's durable data
directory**, and it is named once:

```rust
// crates/overdrive-core/src/vm/config.rs, beside RootfsPlan
/// The platform-owned directory holding the per-launch rootfs clone index.
/// The SINGLE derivation both composition sites call — never re-derived
/// independently. (D3f: independent re-derivation is the defect this closes.)
#[must_use]
pub fn clone_index_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("vm").join("clone-index")
}
```

The in-tree precedent for exactly this shape is `RedbViewStore::resolve_path
(data_dir) -> data_dir.join("reconcilers").join("memory.redb")`, which is
likewise the sole derivation consumed by both its open site and its error
site. The two call sites here are in different functions and cannot share a
`let`, which is precisely why the derivation is a function rather than a local:

- `compose_vm_driver` gains a `clone_index_dir: PathBuf` parameter, stored on
  `VmHostLayout` as a **sixth field**, `clone_index_dir: PathBuf`. It is
  genuinely node-invariant — the same property that let `cgroup_root`,
  `run_dir_root`, `arch`, `vcpus` and `confinement` survive § D3a.
- `RealVmHostState::new`'s third argument stops being the `/run` literal and
  becomes the same `clone_index_dir(&config.data_dir)` value. The parameter is
  renamed `index_dir` to stop naming a staging area it never was.

**`data_dir`, not `/run`.** The index must survive a host reboot or it cannot
serve the reboot-orphan case § D7 designs it for; `/run` is tmpfs.
`ServerConfig.data_dir` is the node's durable root (XDG
`~/.local/share/overdrive` by default), already the home of `intent.redb`,
`workflow-journal.redb` and `reconcilers/memory.redb`. Nothing else about
`ServerConfig` changes.

### D3h — What `RealVmHostState` does instead, and what stays exactly as it is

Two method bodies change. **No trait signature changes**, so § D7 item 3's
"`VmHostState::observe()` is the hydration seam — one call returning a plain
`VmHostObservation`" survives verbatim, and #197's future generalisation is
still a refactor of an existing seam.

- **`observe_clones`** walks `index_dir` exactly as it walks `staging_dir`
  today, parses the same filename, and takes the mapped path from
  `read_link(entry.path())` instead of `entry.path()`. A dangling link still
  yields an entry — that is required, per D3f's crash table. An entry that is
  not a symlink is skipped and logged; this is a greenfield single cut, so no
  migration reads pre-existing regular files there.
- **`discard_artifacts`** stops re-deriving the clone path. It `read_link`s the
  index entry, removes the **target** first, then the **link** — mirroring
  `stop`'s ordering so that an interrupted disposal always leaves the
  self-healing residue rather than the invisible one. Both removals stay
  `NotFound`-tolerant. The run-directory removal above it is unchanged.

**`discard_artifacts` keeps its `(&self, alloc: &AllocationId)` signature and
does *not* take the observed path as a parameter** — and § D7 already supplies
the reason, so this is not a new rule: *"an observation carried from the diff
into the plan goes stale between emit and execute."* DD-5 pins these `Action`s
to `alloc_id` and nothing else; the executor re-observes. Re-reading one
symlink is the cheapest possible instance of that rule.

**`VmReclamation` covers this completely, and gains nothing.** Verified against
the shipped reconciler rather than assumed: `plan_reclamation` already unions
`actual.host.clones.keys()` into its host-id set, already treats
`clones.contains_key(&alloc_id)` as a VM-exclusive trigger for the
`desired == None` arm, and already routes the `terminal` arm unconditionally.
`VmReclamationState`, `VmReclamationView`, `Action::ReclaimAllocation`,
`Action::DiscardStrandedArtifacts`, both executors, the boot-epoch drive and
the sweep interval are **all unchanged**. The reconciler was never the problem;
it was being fed an empty surface.

### Rejected alternatives

- **Enumerate operator-named directories derived from intent.** Rejected on
  three counts, the second decisive. (a) It inverts the port's layering:
  `observe()` is the "what is" hydration seam and takes no arguments; feeding
  it desired state makes a driven observation port consume intent. (b) **It is
  incomplete in a routine case.** An operator who edits `[vm] rootfs` to a new
  path — or deletes the workload — removes the old directory from intent while
  the old clone is still on disk. Those are exactly the `desired == None` rows
  `plan_reclamation`'s disposal arm exists for, so the arm would be
  structurally unreachable for the very inputs it was written to handle.
  (c) It walks operator directories of unbounded size on the sweep cadence.
- **Persist the clone *path* as durable per-allocation state (a row, a `View`
  field, an rkyv envelope).** Rejected. It is a heavier mechanism than a
  symlink for the same information; it mints a serialisable type and therefore
  a schema-evolution obligation; and per `development.md` § "Persist inputs,
  not derived state" a persisted path is a cache of `RootfsPlan::for_alloc`'s
  naming rule that goes stale the moment that rule changes. D3f's record
  deliberately carries **no** path *string* — the symlink is a pointer into the
  real resource, resolved at read time, not a remembered derivation.
- **Constrain clone placement to a single platform-owned directory.**
  Rejected **at the time** as impossible, on the reasoning that FICLONE is
  intra-filesystem and the master's filesystem was operator-chosen per § D3a, so
  no single directory could serve every master; ADR-0082 § D2 gap 3 said as much
  ("staging into `/run` fails `EXDEV`"), and the viable *N*-directory variants
  still required an enumeration mechanism. **Superseded by the ADR-0082 fourth
  amendment (2026-08-18, § (c-fix.2) — see § D3a's amendment note):** clone
  placement IS now constrained to a single platform-owned directory
  (`clone_staging_dir(data_dir)`), made possible by the companion constraint the
  amendment adds — **the rootfs master must reside on the VM data filesystem** —
  so the one staging root is always intra-filesystem with the master, and a
  foreign-filesystem master fails closed (`ConfinementUnavailable { control:
  UidDrop }`, FICLONE `EXDEV`) rather than needing an *N*-directory scheme. The
  single-directory placement is what removes the operator-dir traverse (B1); it
  does **not** disturb § D3f's index, which still records `clone_dest` wherever it
  lives and now enumerates one platform-owned directory instead of chasing
  operator paths.
- **Accept the leak with a documented bound.** Rejected: there is no bound.
  One clone per crashed or restart-orphaned launch, unbounded over the node's
  lifetime, each growing toward full rootfs size, on a filesystem the operator
  depends on. SD-1's own triage names "SD-2's unbounded-over-lifetime clone
  leak" as a founding motivation for the reconciler.

### § D7 correction — two statements the first 2026-08-17 amendment falsified

§ D7 was **correct as authored**: it was written against a single node-level
artifact directory, where the observed clone directory and the staged clone
directory were the same place. § D3a invalidated that premise and did not carry
the correction through. Two statements are corrected here rather than left to
mislead:

1. **"every per-launch clone in the image directory"** (§ D7, *Observe three
   surfaces*). There is no "the image directory" after § D3a. Read instead:
   *every per-launch clone reachable through the platform-owned clone index
   (§ D3f), whose filename carries the allocation id.*
2. **"a reboot-orphaned VM is caught by its clone, which is the only surface
   that survives a host reboot"** (§ D7) and **"The same pass sweeps
   reboot-orphaned clones"** (§ D7). Between § D3a and this amendment these
   were false in *both* directions simultaneously — the enumerated directory
   was `/run` (tmpfs, does not survive a reboot, and by ADR-0082 § D2 gap 3
   cannot hold a clone at all), while the clones that genuinely do survive a
   reboot sat unenumerated on the operator's persistent filesystem. Both
   statements become true again under § D3f–D3h, because the index is durable
   (`data_dir`) and the link's lifetime contains the clone's.

**The three-surface model itself stands and is not widened.** The clone index
is the *index for* surface three, not a fourth surface: it carries no state a
sweep must reconcile independently, and it is disposed of by the same
`discard_artifacts` call as the clone it points to. § D7's attributability
ruling is likewise unchanged — the cgroup scope stays shared with exec
allocations and unattributable-therefore-left-alone; the run directory and the
clone stay VM-exclusive.

### D3i — `run_with_kek` is ratified, ungated, and `run` collapses onto it

With its artifact parameter deleted, `overdrive-cli`'s
`run_with_dataplane_and_vm_artifacts` / `run_with_vm_artifacts` pair collapsed
to a single wrapper the crafter named **`run_with_kek`**, under the naming
authority DISTILL's activation plan gave step 03-07. § D3a deleted the two
entrypoints but did not name the survivor. It is ratified here:

```rust
// crates/overdrive-cli/src/commands/serve.rs — NOT #[cfg]-gated
pub async fn run_with_kek(
    args: ServeArgs,
    kek: Arc<dyn overdrive_core::ca::kek::Kek>,
) -> Result<ServeHandle, CliError>
```

**It is not redundant with `run`, and the reason is load-bearing.** `run(args)`
constructs the production KEK provider inline
(`Arc::new(overdrive_host::ca::SystemdCredsKeyring::new())`) and is the sole
production entrypoint, called from `main.rs`. `run_with_kek` takes the KEK as a
parameter. That is the `Clock` / `Transport` / `Entropy` discipline
`development.md` § "Port-trait dependencies" mandates — *"Required, not
defaulted, at the call site"* — applied at the binary boundary, where the
production wiring legitimately lives in `run` and the injection sibling
legitimately lives beside it. Its two callers pass `SimKek` to dodge the
production provider's cold-boot refusal, which is the same hazard recorded in
project memory as the cause of a prior missed cold-boot regression.

**Their bodies, however, are redundant, and that is what gets collapsed.**
Both delegate byte-identically to `run_inner(args, None, kek, |c| c)`. `run`
therefore becomes:

```rust
pub async fn run(args: ServeArgs) -> Result<ServeHandle, CliError> {
    run_with_kek(args, Arc::new(overdrive_host::ca::SystemdCredsKeyring::new())).await
}
```

One body, not two. The near-duplicate cannot drift because it no longer exists.

**Gating: ungated, deliberately.** It matches the sibling `run_with_dataplane`,
which is likewise ungated with zero production callers. The `#[cfg]` it lost
was never about the entrypoint — it existed only because the deleted
`VmBootArtifacts` *type* in its signature was gated. And unlike
`ServerConfig.vm_artifacts`, a caller-supplied KEK is **not** a state only a
test seam can produce: `Kek` is a real port with a real production binding, and
`run` is that binding's composition site. CLAUDE.md § "Ground the premise" is
therefore satisfied rather than dodged. `run_with_dataplane_and_vmm_override`
**stays** `#[cfg(feature = "integration-tests")]` — § D8's adapter-substitution
fault seam has no production analogue and is untouched.

**Naming asymmetry, observed and accepted so it is not "fixed" later.**
`run_with_dataplane(args, dataplane, kek)` also takes a KEK without naming it,
so the family's implicit rule is "name what you inject *beyond* the mandatory
KEK" — under which this wrapper injects nothing extra. `run_with_kek` is kept
regardless: it names exactly what it takes, it is the minimum-delta name, it
has landed with two callers, and a rename buys nothing but churn.

### Consequences

- **Positive.** § D7's clone surface becomes operative in production for the
  first time. An allocation that ends without `stop` — natural exit, guest
  crash, or a control-plane restart that loses the in-memory `RootfsPlan` — has
  its clone reclaimed by the reconciler that was already designed to do it.
- **Positive.** The two-independent-derivations defect is removed, not
  re-synchronised: `clone_index_dir` is the one derivation, and the clone's own
  location is recorded rather than recomputed. A future move of `clone_dest`
  cannot silently blind the sweep again.
- **Positive.** The index moves from tmpfs to `data_dir`, so the reboot-orphan
  case § D7 designs is reachable rather than notional.
- **Positive.** `VmDriver::stop`'s in-memory fast path is unchanged and stays
  the common case; the sweep is the backstop it was always meant to be.
- **Negative, and stated.** A symlink is a second filesystem artifact per live
  VM. It is bounded (one per launch, a path-sized inode), self-cleaning through
  the same `discard_artifacts` call, and any residue is a dangling link the
  next sweep disposes — but it is not nothing, and the create-before /
  remove-after ordering is a discipline a future edit to `VmDriver` can break.
  A test owns that ordering directly (S-VM-85) precisely because a comment
  would not hold it.
- **Superseded by the ADR-0082 fourth amendment (2026-08-18).** As authored this
  read: *"the clone still lands in the operator's directory, which may still be
  read-only or shared."* That is no longer true — the fourth amendment
  (§ (c-fix.2) — see § D3a's amendment note) relocates the clone into the
  platform-owned `clone_staging_dir(data_dir)`, so the read-only / shared
  operator-directory concern § D3a raised is resolved for the clone (the operator
  directory is no longer written to or traversed). What remains is the companion
  precondition — the **rootfs master must reside on the VM data filesystem**, with
  a foreign-filesystem master failing closed (`ConfinementUnavailable { control:
  UidDrop }`, FICLONE `EXDEV`); § D3b's narrowing of `Vmm::probe`'s reflink proof
  and S-VM-94 / step 03-08's sixth criterion continue to own the same-filesystem
  non-reflink fail-closed. The clone-index reclamation of §§ D3f–D3h is unchanged
  by the relocation — `index_link` records `clone_dest` wherever it lives.
- **Negative.** `VmHostLayout` gains a field one step after § D3a removed two,
  and `compose_vm_driver` gains a parameter. The justification is the same test
  § D3a applied: `clone_index_dir` is node-invariant, so it belongs on the
  node-invariant struct.
- **Neutral.** No `VmHostState` signature, no `Action` variant, no
  `TransitionReason` variant, no `VmmError` variant, no `Vmm` method, no
  reconciler `State` / `View`, no rkyv envelope and no operator surface
  changes. `plan_reclamation` is untouched.

Recorded in feature DWD-26. Delivered by roadmap step 03-09.

---

## Amendment 2026-08-18 — Volumes (Slice 04) cut from this feature; §§ D3 / D5 / D8 volume decisions superseded

**Decision-maker:** Morgan (nw-solution-architect, DESIGN wave). **Mode:** propose.
**Ratified by the user 2026-08-18.** Tags: vm-driver, volumes, descope, GH-42, GH-97,
GH-43, GH-22.

**What changed, and why.** `[[vm.volume]]` (§ D3 `VmPayload.volumes`, § D5's storage
transition reasons, § D8's `virtiofsd` sandbox ruling) was designed as a **virtiofs
host↔guest bind-mount**: a host `source` directory shared into the guest at `target`,
host and guest seeing the same bytes (S-VM-55: *"byte-identical in the operator's host
source directory"*). The user has ruled this the **wrong mechanism and the wrong name**
and removed volumes from this feature entirely. Three grounds, recorded faithfully:

1. **A real persistent volume is block-device-shaped, not a virtiofs share — the
   opposite mechanism.** A platform-managed, guest-owned persistent volume is
   `overdrive-fs` (GH #97, Phase 6.13). Its own spike evidence (P9–P11, banked on #97)
   states the guest should see a **block device (ext4 / vhost-user-blk), NOT a
   virtiofs / vhost-user-fs / FUSE mount** — the inverse of Slice 04's design. #97 was
   **explicitly scoped OUT of #42** by the user's 2026-08-10 ruling (*"boot a VM through
   serve + deploy. Nothing else … the chunk store … belong[s] to #96 / #97 / #100 and
   [is] explicitly out of this feature's design"*).
2. **The virtiofsd mechanism is a separate, later concern.** GH #43 ([3.6] virtiofsd
   lifecycle management + cross-workload volume sharing) owns the `virtiofsd`
   supervision, socket-readiness, and sharing mechanism that Slice 04 (steps
   05-02 … 05-06) was going to build.
3. **The headline use case is fictional on the production target.** US-VM-8's frame —
   *"Ana reads the output on the host with `ls` / `cat`"* — assumes an operator shell on
   the node. Overdrive nodes run an **immutable appliance OS with no operator shell**;
   the artifact is node-local and does not survive the node or a reschedule; the correct
   artifact sink is the **object store (Garage, GH #22)** or **`overdrive-fs` (#97)**. So
   volumes are **deferred, not renamed or kept.**

**Superseded (volume-only — removed from this feature's scope; deferred to #97 / #43 / #22):**

- **§ D3 — `VmPayload.volumes: Vec<VmVolume>`** and the `VmVolume` value are removed;
  `VmPayload` carries **no volume field**. The persisted V2 `Vm` payload was already
  authored *"minus `volumes`"* (Amendment 2026-08-12) — that is now the permanent shape,
  no longer a Slice-04-pending deferral, so **no rkyv envelope change follows from this
  cut** (the V1→V2 bump already landed at step 01-02 and is unaffected).
- **§ D5 — the five volume `VmStartFailure` variants** `VolumeSourceNotFound`,
  `StorageDaemonAbsent`, `GuestMountFailed`, `StorageSocketTimeout`,
  `StorageSandboxUnavailable` and their `TransitionReason` mappings (**table rows 8–12**:
  `VmVolumeSourceNotFound`, `VmStorageDaemonAbsent`, `VmGuestMountFailed`,
  `VmStorageSocketTimeout`, `VmStorageSandboxUnavailable`) are removed, along with the
  `ConfinementControl` values used only by them (none — `ConfinementControl` stays whole;
  it is shared with § D5 row 6 `VmConfinementUnavailable`, which is KEPT).
- **§ D5 — mid-run row 14** `VmStorageDaemonDied`, the `ExitEvent.storage_daemon_died`
  field, the `StorageDaemonDeathFacts` value, and the ahead-of-`ExitKind` precedence
  check are removed. `ExitEvent` retains **only** its `oom` field (row 13). The header
  amendment notes dated 2026-08-11 (gap-1, row 14) and the 2026-08-11 `virtiofsd` sandbox
  rulings are superseded by this amendment.
- **§ D8 closing ruling (S-VM-67)** — the `--sandbox=namespace` argv-layer assertion and
  the *"this feature mints no storage-daemon supervision port"* negative decision are
  **moot**: there is no `virtiofsd` in this feature at all. (`virtiofsd` sandbox posture
  is now wholly #43's / #258's when that work is designed.)
- **Consequences** — the *"S-VM-67 has no seam this ADR pair supplies"* bullet and the
  *"`ExitEvent` gains a second additive field (`storage_daemon_died`)"* bullet are
  superseded.

**KEPT (shared with the volume-free path — explicitly NOT removed):**

- **§ D5 rows 1–7** (`VmKernelNotFound`, `VmRootfsNotFound`, `VmHypervisorAbsent`,
  `VmBootDeadlineExceeded`, `VmKernelFormatUnsupported`, `VmConfinementUnavailable`,
  `VmGuestExitUnreported`), **row 13** (`VmOutOfMemory`, mid-run OOM), and **row 15**
  (`VmGuestCommandDispatchFailed` — READY arrived but the `EXEC` beacon delivery failed
  before `Running`; **not** a volume concern). These are core VM boot / confinement /
  memory diagnoses and are untouched.
- **`ExitEvent.oom`** (ADR-0082 § D8) and its `handle_exit_event` precedence check — row
  13 — stay.
- **`ConfinementControl`** (all six controls) — shared with row 6 `VmConfinementUnavailable`.
- **§§ D1, D2, D2a, D3 (payload minus volumes), D4, D6, D7, D8 (the `vmm_override` seam),
  and all D3a–D3i amendments** — unchanged.

**K3 still holds.** § D5's *"≥ 4 distinct diagnoses"* count survives the cut with margin:
**nine** distinct VM diagnoses remain — eight start-time (rows 1–7 and row 15) plus one
mid-run (row 13 `VmOutOfMemory`) — against the ≥ 4 floor, before the four Exec classes are
counted. Cutting volumes does not threaten K3.

**Companion / plan changes made in the same pass (recorded for cross-reference):**

- `feature-delta.md` — US-VM-8 / US-VM-9 and the `[D8]` … `[D8g]` family marked descoped
  (deferred → #97 / #43 / #22), dated 2026-08-18; the narrative is annotated, not deleted.
- `distill/test-scenarios.md` — S-VM-55 … S-VM-68 annotated descoped; S-VM-72's
  `shared=on` half withdrawn.
- `deliver/roadmap.json` — Phase 05 (steps 05-01 … 05-06) removed; step 06-03 reduced to
  single- (private-) memory-backing sizing parity, its `Driver::resize` portion kept, and
  its `05-01` dependency dropped.
- The already-landed step **05-01** (commit `ebef5c27`) is reverted under the single-cut
  rule (no stubs, no shims): `VmVolumeInput` / `VmInput.volume` in `workload_spec.rs`,
  `VmVolume` / `MemoryBacking` (the derived `shared=on`) in `vm/config.rs`, the whole
  `vm_volumes_and_storage_daemon.rs` test file, its `mod` line in `integration.rs`, and
  the two S-VM-62 volume tests appended to `vm_spec_driver_table_dispatch.rs`. `ebef5c27`
  is purely additive (+607 / −0) and 100% volume-only, so a straight revert is clean.

**Out of scope for this amendment (surfaced, not actioned).** ADR-0082 carries
forward-references to volumes as a Slice-04 concern — VmConfig deliberately holds *no*
volume field, but its `--memory shared=on` forward pointer (VmConfig memory docstring) and
`virtiofsd`-sidecar mentions now point at deferred work. Those references are stale but
harmless (they describe what a future slice *would* do); correcting them is a separate edit
outside this amendment's named scope, surfaced to the orchestrator.

Recorded in feature DWD (2026-08-18 volumes cut). Supersedes the 05-01 plan step and the
Slice-04 volume decisions across §§ D3 / D5 / D8.
