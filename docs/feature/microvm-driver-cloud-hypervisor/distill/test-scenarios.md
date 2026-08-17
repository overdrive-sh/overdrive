<!-- markdownlint-disable MD024 -->

# Test Scenarios — microvm-driver-cloud-hypervisor

**Wave**: DISTILL
**Feature**: microvm-driver-cloud-hypervisor (GH #42)
**Date**: 2026-08-11
**SSOT**: `docs/feature/microvm-driver-cloud-hypervisor/feature-delta.md` (DISCUSS + three DESIGN dispatches) · ADR-0081 (three Ending Classes) · ADR-0082 (`Vmm` port + `VmConfig`) · ADR-0083 (`DriverRegistry` + `VmReclamation`) · `docs/product/architecture/brief.md` §§ 89–114 · `spike/findings.md` + `spike/wave-decisions.md` (PROMOTE)

All scenarios below are specification-level GIVEN/WHEN/THEN. Per
`.claude/rules/testing.md`, **no `.feature` files** — the DELIVER crafter
translates each scenario into a Rust `#[test]` / `#[tokio::test]` function,
RED-scaffolded per `.claude/rules/testing.md` § "RED scaffolds"
(`#[should_panic(expected = "RED scaffold")]`). Scenarios are grouped by
slice, mirroring the story-to-slice map DESIGN already fixed
(US-VM-1→01, US-VM-2/US-VM-6→02, US-VM-3/US-VM-4/US-VM-7→03,
US-VM-8/US-VM-9→04, US-VM-5→05), plus one cross-cutting section for the
`VmReclamation` reconciler (SD-1, Bar 2 — a node-level component with no
single owning story).

**One scenario is marked `@walking_skeleton`**: S-VM-01. It is the only
scenario driven end-to-end through `overdrive serve` + `overdrive deploy`
with a **real** Cloud Hypervisor VMM and a real guest kernel — every other
scenario either drives the same CLI path with a different fixture, or
enters at a narrower driving port (a reconciler tick, a pure function, a
driven-port equivalence test) per this project's four-tier discipline.

**46 scenarios carry `@requires-kvm`** — the capability class distinct from
`@tier3`/`@real-io`. `spike/findings.md` § "The nested-virt stall — SETTLED
2026-08-10" measured a real asymmetry: bare-metal x86_64 booted 12/12
(median 0.744s), while nested-aarch64 (the standard macOS dev Lima VM)
stalled ~1 in 3 — "a stall and a real regression are indistinguishable,"
and the spike explicitly deferred the gating decision to "Slice 01's first
integration test." `@tier3`/`@real-io` means *"real infra that works
inside Lima"* (netns, cgroups, subprocesses, cgroupfs, real filesystem
probes) — that predicate does not distinguish it from booting an actual
guest kernel under KVM, which is a narrower and flakier capability class on
a nested-virtualization host. `@requires-kvm` is applied to every scenario
whose own Given/When/Then requires a real `cloud-hypervisor` process to be
spawned with intent to boot a guest kernel (including scenarios where the
guest deliberately never reaches userspace — the boot ATTEMPT itself is
what exercises KVM and is subject to the stall) — see DWD-17 in
`wave-decisions.md` for the full classification method, the per-scenario
disposition, and the scenarios flagged genuinely ambiguous. `@tier3`
scenarios that are real I/O but do **not** spawn a real guest-booting
`cloud-hypervisor` process (pure-function/parse rejections that already
carry `@tier1`, node-boot capability probes, `SimVmm`-injected fault
scenarios, adapter-equivalence tests over generic cgroupfs/filesystem
primitives, and pre-spawn artifact-validation rejections) do **not** carry
`@requires-kvm`.

**`@requires-kvm` ⇒ the `kvm-tests` Cargo feature.** The concurrent
`deliver/roadmap.json` pass has pinned the Rust-side mechanism this tag
consumes: a scenario carrying `@requires-kvm` compiles only in a test
file gated behind `--features integration-tests,kvm-tests` (declared
narrowly on `crates/overdrive-host` and `crates/overdrive-cli` only — not
workspace-wide, unlike `integration-tests` — because `kvm-tests` gates no
mutation-testing surface and has no forcing function requiring a
universal per-crate declaration). A runtime preflight (real `/dev/kvm`
open + `systemd-detect-virt`) is the secondary defense, for the case the
feature is deliberately enabled on an incapable host; it fails loudly and
named, never a silent skip. See DWD-17 (and its DWD-18 addendum) in
`wave-decisions.md` for the full binding rationale, the naming
reasoning, and the reconciliation of this classification against the
roadmap's file-level gate.

---

## Driving Ports

| Port | Kind | Location | Exercises |
|---|---|---|---|
| `overdrive deploy <spec.toml>` — direct CLI handler call (`overdrive_cli::commands::deploy::deploy`) against a REAL in-process `overdrive serve`, run under `cargo xtask lima run --` as root | Driving (user-facing) | `crates/overdrive-cli`, per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess" (firm rule — no `Command::new(CARGO_BIN_EXE_overdrive)` anywhere in this feature) | S-VM-01…05, S-VM-09, S-VM-11…15, S-VM-19, S-VM-33…66, S-VM-68, S-VM-74, S-VM-75, S-VM-81 (Tier-3 cases; **excludes S-VM-67**, moved to the pure-function row below — DWD-13; **explicitly names S-VM-09/S-VM-19** — DWD-16 closes the sub-range-gap omission DWD-15 flagged). The REAL OS-level subprocess in every one of these is `cloud-hypervisor` itself, spawned by `VmDriver`/`CloudHypervisorVmm` inside the real `overdrive serve` process — that is what makes them Tier-3/`@real-io`, not how the CLI layer is invoked |
| `overdrive workload describe <id>` / `overdrive job stop <id>` | Driving (user-facing) | `crates/overdrive-cli` | Read-side assertions on every Tier-3 scenario; S-VM-46/47 (stop verb) |
| `VmDriver::start` / `VmDriver::stop` — **component-scope acceptance case against `SimVmm`, injected at the `Vmm` port boundary** | Driving (internal, component scope — the enforcement vehicle ADR-0082 §D4 names by name: *"`VmDriver::stop`'s edge cases … are therefore asserted by a `VmDriver`-level acceptance case against `SimVmm`, named here so the move does not quietly shed the enforcement it was partly justified on"*) | `crates/overdrive-worker` | S-VM-76. Carved out of the "always reached through the CLI/serve pair" default because `vmm_equivalence.rs` drives the `Vmm` port only and structurally cannot reach `VmDriver::stop`'s relocated guest half (ADR-0082 §D4) — this is `SimVmm` injected at a port boundary per system constraint 1, not a bypass of it |
| `Vmm` port (`CloudHypervisorVmm` / `SimVmm`) | Driven (adapter-under-test for equivalence only) | `crates/overdrive-host`, `crates/overdrive-sim` | `vmm_equivalence.rs` (S-VM-90); `vmm_ficlone_per_launch.rs` (S-VM-94, real non-reflink substrate) |
| `VmHostState` port (`RealVmHostState` / `SimVmHostState`) | Driven (adapter-under-test) | `crates/overdrive-host`, `crates/overdrive-sim` | `vm_host_state_equivalence.rs` (S-VM-91) |
| `CgroupAccounting` port (`RealCgroupAccounting` / `SimCgroupAccounting`) | Driven (adapter-under-test) | `crates/overdrive-host`, `crates/overdrive-sim` | `cgroup_accounting_equivalence.rs` (S-VM-93) |
| `plan_reclamation(desired, actual) -> Vec<Action>` (pure function) | Driving (internal, port-to-port at function scope) | `crates/overdrive-core/src/reconcilers/vm_reclamation.rs` | S-VM-21…32, S-VM-78, S-VM-80 (in-memory half; **excludes S-VM-77/S-VM-79** — moved to the `overdrive-control-plane` row below, DWD-16); Tier-3 half drives through `overdrive serve`'s convergence loop |
| The exit observer's loop body (`worker/exit_observer.rs:204-371`, claim-release across `RetryOutcome` arms) / `execute_reclaim_allocation` (the `ReclaimAllocation` action executor, `action_shim/`) | Driving (internal, component scope) | `crates/overdrive-control-plane` | S-VM-77, S-VM-79 — moved here from the `plan_reclamation` row above (DWD-16): both driving ports are `overdrive-control-plane`-resident production code, structurally unreachable from an `overdrive-core` test (`overdrive-control-plane` depends on `overdrive-core`, never the reverse) |
| `VmReclamation` reconciler tick (`ReconcilerRuntime`) | Driving (internal) | `crates/overdrive-core` runtime, composed in `overdrive-control-plane` | S-VM-22, S-VM-24, S-VM-25 Tier-3 shapes |
| DST harness (`cargo dst`) + `Invariant` catalogue | Driving (internal, Tier 1) | `crates/overdrive-sim` | S-VM-88 (`VmReclamationConverges`), S-VM-24 in-memory shape (`SupervisedVmSurvivesEveryTick`), S-VM-87 (`VmReclamationIdempotentSteadyState`), S-VM-89 (`EndingInFlightIsNeverReclaimed`) |
| `KernelImage::validate`, `DiskAttachment::to_disk_arg`, `VmConfinement::seccomp_arg`, `MemoryPlan::derive`, `VmConfig::rlimit_fsize`, `SupervisionSet::reclamation_authorised` (pure functions) | Driving (unit scope) | `crates/overdrive-core` | S-VM-08, S-VM-16, S-VM-17, S-VM-20, S-VM-63, S-VM-73, S-VM-92 |
| Storage daemon's launch-argument rendering site (pure function; mirrors `DiskAttachment::to_disk_arg`'s D2.1 shape — private fields, one rendering site; **exact type NOT yet pinned by any ADR**, Slice 04's own, DELIVER names it at RED phase, per ADR-0083 §D8's closing amendment) | Driving (unit scope, forward-specified — DWD-13) | `crates/overdrive-core` (exact file TBD) | S-VM-67 |

---

## Slice 01 — `vm-job-boots-and-exit-code-is-honest` (walking skeleton)

Consumes: US-VM-1 (all 5 ACs + Engineering Constraints), ADR-0082 D1–D3/D5/D8,
ADR-0083 D1–D5, contradictions C-1/C-2/C-3/C-5/C-7, KPIs K1/K2/K4/K6/K7 (item
5–6 half), the D-3 fold-in (`CgroupAccounting`, `TransitionReason::VmOutOfMemory`).

### AC-01: A VM workload runs to completion and its real exit code reaches the operator

#### S-VM-01: A VM workload runs to completion and its exit code reaches the operator

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, real Cloud Hypervisor VMM)
**Tags**: `@contract-shape:bounded-change` `@walking_skeleton` `@driving_port` `@happy_path` `@ac-01` `@tier3` `@real-io` `@requires-kvm` `@kpi:K1` `@kpi:K4` `@kpi:K6`

```gherkin
Given a kernel and an ext4 rootfs are staged on the host, and the rootfs's
  guest command exits 0
And Ana has written render.toml declaring [job] and [vm] naming those artifacts
When she runs "overdrive deploy render.toml" against a running "overdrive serve"
Then a real Cloud Hypervisor VM boots and runs her command in the guest
And "overdrive workload describe batch-render" shows Terminated with completed
  exit code 0
And Running was reached no earlier than the ready beacon over vsock arrived
```

**Crafter notes**: The ONLY scenario in this feature driven with a real VMM
and a real guest kernel under `cargo xtask lima run --`. No test installs,
binds, programs, or supplies anything `run_server` does not supply itself —
`DriverRegistry` discovers `cloud-hypervisor` itself; the test does not
hand-compose `VmDriver`. K6 (p50 ≤3s / p99 ≤10s over ≥20 deploys) is a
companion timing assertion on this same driving path, not a separate
scenario — record wall-clock per run.

#### S-VM-02: A non-zero guest exit code is reported, never the hypervisor's

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@walking_skeleton` `@error_path` `@ac-01` `@tier3` `@real-io` `@requires-kvm` `@kpi:K1`

```gherkin
Given Ana has deployed a VM workload whose guest command exits 7
When the guest command finishes and the VM shuts down
Then "overdrive workload describe" shows the allocation Failed with exit code 7
And the host cloud-hypervisor process's own exit code (0) appears nowhere in
  the reported outcome
```

**Crafter notes**: The discriminating case — the guest exits non-zero while
the VMM process exits 0. Mutation target: any code path deriving `ExitKind`
from the VMM's `wait()` result must be killed by this case (brief §103, "No
code path derives `ExitKind` from the `cloud-hypervisor` process's own exit
status").

#### S-VM-03: A guest that never starts is never reported Running

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@walking_skeleton` `@error_path` `@ac-01` `@tier3` `@real-io` `@requires-kvm` `@kpi:K2` `@guardrail`

```gherkin
Given Ana has deployed a VM workload whose rootfs has no working init
When the boot deadline (VM_BOOT_DEADLINE = 30s) elapses without a ready beacon
Then the allocation transitions Pending to Failed
And the allocation's transition history never contains a Running state
```

**Crafter notes**: K2 guardrail — must stay 0 forever. Poll transition
history, not just the final state, to catch a transient false-Running.

#### S-VM-04: A VM workload deploys through the same verb as a process workload

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@walking_skeleton` `@happy_path` `@ac-01` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana already deploys process workloads with "overdrive deploy <spec>"
When she deploys a spec whose driver table is [vm] instead of [exec]
Then the workload is accepted and scheduled with no new verb and no new flag
```

#### S-VM-05: The platform contains the hypervisor it started

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process, mTLS-composed `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@walking_skeleton` `@happy_path` `@ac-01` `@tier3` `@real-io` `@requires-kvm` `@kpi:K7`

```gherkin
Given Ana has deployed a VM workload on a node running the PRODUCTION
  mTLS-composed composition (dataplane_override unset)
When the allocation reaches Running
Then /proc/<vmm-pid>/cgroup resolves to that allocation's workload scope
And /proc/<vmm-pid>/ns/net is the inode of /var/run/netns/<spec.netns>, not
  the host netns
And the hypervisor's seccomp filter is non-default (per-thread check, S-VM-09)
```

**Crafter notes**: MUST run against an mTLS-composed `overdrive serve` — an
mTLS-uncomposed boot leaves `spec.netns = None` and would satisfy a
conditionally-worded assertion with zero placement code written (the GH #248
/ ADR-0074 trap this feature deliberately reproduces to prevent it, per
US-VM-1 AC 5). `<vmm-pid>` resolves through the allocation's cgroup
`cgroup.procs`, never a driver-internal handle.

#### S-VM-74: A VM allocation reaching Running gets no mTLS intercept state — ungated-and-succeeding would be a silent false confidentiality claim

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process, mTLS-composed `overdrive serve`)
**Tags**: `@contract-shape:unbounded-preservation` `@error_path` `@ac-01` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has deployed a VM workload on an mTLS-composed "overdrive serve"
When the allocation reaches Running
Then no mTLS intercept state exists for that allocation's veth (no bound
  transparent listener, no installed TPROXY rule, on either leg)
And this holds whether or not the allocation is confined successfully --
  the gate is on the driver type, not on the confinement outcome
```

**Crafter notes**: ADR-0083 §D2a(c) — `MtlsInterceptWorker::start_alloc`'s two
call sites gate on `state == AllocState::Running` and
`mtls_worker.is_some()`, and (pinned by this decision) on
`spec.driver.driver_type() == DriverType::Exec`, not on anything else.
S-VM-05 catches only the fail-closed arm (a `VmDriver` whose install would be
attempted and refused, per the *original* docstring predicate falsified by a
second driver). This scenario catches the OTHER failure direction: an
ungated-and-succeeding install would host-socket-intercept a veth the guest's
TCP never traverses (traffic terminates *inside* the guest, per GH #222) and
present it as mesh-enrolled when it is not -- a silent false confidentiality
claim, not a refusal. `provision_and_inject_netns` is NOT gated (the VM still
gets its netns slot) -- only the intercept install is.

### AC-02: The parser accepts exactly one driver table

#### S-VM-06: Both [exec] and [vm] present is rejected

**Driving port**: `WorkloadSpecInput::from_toml_str()` (pure function — in-process TOML parse boundary, no subprocess, no `overdrive serve` needed)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-02` `@tier1` `@in-memory`

```gherkin
Given a spec declares both an [exec] table and a [vm] table
When the operator submits it via "overdrive deploy"
Then the spec is rejected with MultipleDriverSections naming both tables
And no allocation is created and no intent is committed
```

#### S-VM-07: Neither [exec] nor [vm] present is rejected

**Driving port**: `WorkloadSpecInput::from_toml_str()` (pure function — in-process TOML parse boundary, no subprocess, no `overdrive serve` needed)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-02` `@tier1` `@in-memory`

```gherkin
Given a spec declares neither an [exec] table nor a [vm] table
When the operator submits it
Then the spec is rejected with MissingDriverSection
```

**Crafter notes**: S-VM-06/07 replace `ParseError::MissingExec`
(`workload_spec.rs:743-745`) per ADR-0083 §D4. `ParseError::MissingExec` is
deleted in the same PR (single cut, no alias).

### AC-03: The hypervisor's syscall surface is filtered and it is never weakened

#### S-VM-08: Seccomp is constructed explicitly and never weakened

**Driving port**: `VmConfinement::seccomp_arg()` (pure function)
**Tags**: `@contract-shape:pure-function` `@property` `@ac-03` `@tier1` `@in-memory` `@mandatory:mutation_target`

```gherkin
Given any VmConfinement value the driver could construct
When seccomp_arg() renders the --seccomp argument
Then the rendered value is always the literal "true"
```

**Crafter notes**: The renderer is the mutation site regardless of enum
cardinality (brief §102, the withdrawn-then-restored `SeccompMode`
reasoning). A mutation flipping the rendered literal must be killed — the
`@mandatory:mutation_target` tag is what enforces "no code path in the
workspace can produce `false` or `log`" (a workspace-negative claim no
port-observable assertion can make from inside the proptest itself); the
`xtask dst-lint` `--seccomp` AST clause (`brief.md` §113) is the
complementary static enforcement — see `distill/wave-decisions.md`'s
dst-lint-clause decision. This is the argv-level half of US-VM-1 AC item 6;
S-VM-09 is the `/proc`-level half.

#### S-VM-09: Seccomp is verified per-thread, not on the thread-group leader

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-03` `@tier3` `@real-io` `@requires-kvm` `@correction:C-5`

```gherkin
Given Ana has deployed a VM workload and the allocation reaches Running
When the platform reads /proc/<vmm-pid>/task/*/status for every CH thread
Then at least the vmm, http-server and vcpu0 threads report a non-default
  Seccomp mode
And a bare read of /proc/<vmm-pid>/status alone (thread-group leader) is
  NEVER used as the sole evidence — it correctly reports Seccomp: 0 on a
  properly-confined CH and must not fail this scenario
```

**Crafter notes**: C-5 — the original Slice 01 AC read
`/proc/<pid>/status`, which **fails against correct behaviour** (spike P5
correction 2). This scenario is the corrected AC, not a new capability.

### AC-04: Adding a VM driver is a schema-evolution event, executed correctly

#### S-VM-10: JobEnvelope V1 decodes correctly after the V2 bump

**Driving port**: golden-bytes fixture roundtrip (pure function, `rkyv`)
**Tags**: `@contract-shape:pure-function` `@property` `@ac-04` `@tier1` `@in-memory` `@mandatory:schema_evolution`

```gherkin
Given the existing FIXTURE_V1 golden bytes for the Job aggregate (untouched)
When they are rkyv-deserialised through JobEnvelope and into_latest() is called
Then the result is byte-identical to the pre-bump V1 projection
And a freshly-constructed WorkloadDriver::Vm(Vm) job round-trips through
  JobEnvelope::V2 unchanged
```

**Crafter notes**: Per `.claude/rules/development.md` § "rkyv schema
evolution" Version-bump procedure — six steps, single commit,
`FIXTURE_V1` never touched. `[[vm.volume]]` rides inside this same V2 (design
closed the "does it need its own bump" open question — brief §100/104: it
does not).

### AC-05: Driver dispatch — the registry IS the composition gate (SD-5)

#### S-VM-11: cloud-hypervisor present and healthy composes the Vm driver

**Driving port**: `overdrive serve` boot (in-process, composition root)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-05` `@tier3` `@real-io`

```gherkin
Given a host with a working cloud-hypervisor binary and a reflink-capable
  VM staging filesystem
When "overdrive serve" boots
Then the driver registry reports DriverType::Vm as supported
And a subsequent [vm] deploy is accepted
```

#### S-VM-12: cloud-hypervisor absent — the node boots, [vm] is rejected at admission

**Driving port**: `overdrive serve` boot + `overdrive deploy`
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-05` `@tier3` `@real-io` `@kpi:K4`

```gherkin
Given a host with no cloud-hypervisor binary installed
When "overdrive serve" boots
Then the node boots successfully with no Vm entry in the driver registry
And a subsequent [vm] deploy is rejected at admission naming the absent
  capability -- not a parse error
```

**Crafter notes**: Capability *absence* is not a fault (SD-5). Must be
distinguished from S-VM-13 by a different code path and a different
`overdrive workload describe`-visible message shape.

#### S-VM-13: cloud-hypervisor present but a capability the host cannot supply is missing — the node refuses to boot

**Driving port**: `overdrive serve` boot
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-05` `@tier3` `@real-io` `@mandatory:mutation_target`

```gherkin
Given a host with cloud-hypervisor installed but missing a capability flag
  the probe requires (--landlock absent below the version floor, the host
  Landlock LSM absent, or /dev/kvm unreachable under the target identity)
When "overdrive serve" attempts to boot
Then the boot refuses with a health.startup.refused event naming the probe
And the failure is uniform in shape with MtlsEnforcement::probe's refusal
```

**Crafter notes**: `Vmm::probe()` injected at the port boundary via `SimVmm`
fault injection (system constraint 1). Scoped, after correction, to the
**capability-flag** fault classes of ADR-0082 §D5 scenarios 2–4
(`LandlockFlagAbsent`, `LandlockLsmAbsent`, `KvmUnreachable`) — for these,
"no genuinely lying host exists in the Lima test envelope" IS true: the Lima
kernel/binary/uid shape cannot be made to lack `--landlock` or the Landlock
LSM or `/dev/kvm` access without swapping the kernel image, which the fixed
test envelope does not do. **The non-reflink fault class (D5 scenario 1) is
NOT injected here** — see S-VM-75, which corrects an earlier false claim
that no genuinely non-reflink host exists in the envelope (it does: a tmpfs
or loopback-ext4 staging directory, real, one mount command away). The
run-root-unusable class (D5 scenario 5) stays `SimVmm`-injected alongside
the capability-flag classes — an unbindable/unwritable run root is a
permissions/mount-table fact the fixed Lima kernel genuinely cannot
reproduce without root-owned interference the harness does not perform.

**Injection seam — RESOLVED by the concurrent DESIGN pass, ADR-0083 §D8.**
`SimVmm` substitutes for `CloudHypervisorVmm` via
`ServerConfig.vmm_override: Option<Arc<dyn overdrive_core::traits::vmm::Vmm>>`,
`#[cfg(feature = "integration-tests")]`-gated on both the declaration and
its one use site in the composition root, resolved *before*
`CloudHypervisorVmm::discover` is called (§D2's snippet, amended). This is
shaped after the already-shipped `mtls_identity_override` — a whole-**port**
-implementation swap (`Arc<dyn Trait>`) — deliberately NOT after
`dataplane_override`'s whole-**subsystem** gate (ADR-0083 §A10 considers and
rejects the `dataplane_override`-shaped `compose_vmm = vmm_override.is_none()`
alternative by name — that is the exact GH #248 / ADR-0074 shape this
scenario's DESIGN reasoning deliberately avoids). Every downstream consumer
(`DriverRegistry`, the exit-observer loop, `alloc_drivers`,
`MtlsInterceptWorker`'s gate, `VmReclamation`) sees `Arc<dyn Vmm>` and is
unaware the seam exists; `.probe()` still runs unconditionally against
whichever adapter is bound — Earned Trust is never skipped for the injected
case. The states this seam injects (`ReflinkUnsupported`,
`LandlockFlagAbsent`, `LandlockLsmAbsent`, `KvmUnreachable`, `RunDirUnusable`)
are ADR-0082 §D5's own catalogued, production-reachable substrate lies, not
states only the seam itself can produce. The same seam covers S-VM-51.
**It does NOT cover S-VM-67** — see that scenario's own crafter note.

#### S-VM-75: cloud-hypervisor present, capability flags all satisfied, but the VM staging directory is genuinely non-reflink — the node refuses to boot

**Driving port**: `overdrive serve` boot, against a REAL tmpfs (or
loopback-mounted ext4) staging directory — no `SimVmm` fault injection
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-05` `@tier3` `@real-io` `@mandatory:mutation_target`

```gherkin
Given a host with cloud-hypervisor installed and every capability flag
  present, whose VM staging directory is REAL tmpfs (or a real loopback-
  mounted ext4 image), neither of which supports FICLONE across the probe's
  source/destination pair
When "overdrive serve" attempts to boot
Then the boot refuses with a health.startup.refused event naming
  ReflinkUnsupported, the directory and the observed fstype
And the refusal comes from an EXECUTED FICLONE ioctl call that returned
  EOPNOTSUPP or EXDEV, never from an fstype string comparison
```

**Crafter notes**: Corrects DWD-02's/the project policy's prior claim that
"no genuinely lying host exists in the Lima test envelope" for the
non-reflink class — that claim was **false**: a tmpfs `TempDir` or a
loopback-mounted ext4 image inside the real Lima root harness this suite
already uses (`overdrive-testing`) IS a genuinely non-reflink real
substrate, no injection needed. ADR-0082 §D5 scenario 1 requires the probe
to be an **executed `FICLONE`**, not an fstype comparison
(`infra/metal/provision.sh:419-430`'s pattern) — this scenario is the one
place in the suite that proves the probe is real by running it against a
substrate that genuinely cannot satisfy it, rather than a `SimVmm` stand-in
asserting the probe *would* refuse. Placement: `overdrive-cli`
`tests/integration/vm_walking_skeleton.rs` alongside S-VM-11…13, using a
real `tempfile::TempDir` mounted tmpfs (or a loopback ext4 image) as the VM
staging root override for this one boot.

### AC-06: The three-way boot race is correct on every arm, including cleanup

#### S-VM-14: The deadline arm leaks nothing

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, `SimVmm`-free — real CH that never beacons)
**Tags**: `@contract-shape:unbounded-preservation` `@error_path` `@ac-06` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has deployed a VM workload whose guest never beacons ready
When VM_BOOT_DEADLINE elapses
Then the allocation is Failed
And no cloud-hypervisor process, no run directory, no rootfs clone and no
  cgroup scope remain for that allocation
And the allocation's supervision claim (taken at step 0 of start) is released
```

**Crafter notes**: The arm an implementation is most likely to leak on
(brief §103). Mutation target: dropping the cleanup on the deadline arm must
be killed.

#### S-VM-15: A guest EXIT report is never overwritten by the VMM's own teardown exit

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-06` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana's guest reports EXIT 7 over vsock
When the cloud-hypervisor process then exits 0 during its own teardown
Then the reported exit code stays 7
And the exit report's arrival is ordered strictly before the ExitEvent emission
```

**Crafter notes**: The Slice-03-flagged ordering hazard — closed by making
the guest report authoritative and drained to completion before emission
(brief §103).

### AC-07: `VmConfig` makes three substrate lies structurally unrepresentable (C-2, C-3, C-7)

#### S-VM-16: Every --disk argument carries image_type=raw unconditionally

**Driving port**: `DiskAttachment::to_disk_arg()` (pure function)
**Tags**: `@contract-shape:pure-function` `@property` `@ac-07` `@tier1` `@in-memory` `@mandatory:mutation_target` `@correction:C-2`

```gherkin
Given any DiskAttachment value (any path, either read_only value)
When to_disk_arg() renders the --disk argument
Then the rendered string always contains "image_type=raw"
And there is no DiskAttachment constructor that omits it
```

**Crafter notes**: Complementary static enforcement — the `xtask dst-lint`
`"--disk"` AST clause (`brief.md` §113: *"never rendered outside
`DiskAttachment::to_disk_arg`"*) is a `xtask/src/dst_lint.rs` unit test, not
a Rust acceptance-test scaffold; see `distill/wave-decisions.md`'s
dst-lint-clause decision for ownership.

#### S-VM-17: An unloadable kernel is rejected with a format error before Cloud Hypervisor ever sees the file

**Driving port**: `KernelImage::validate(path, arch, header)` (pure function)
**Tags**: `@contract-shape:pure-function` `@property` `@error_path` `@ac-07` `@tier1` `@in-memory` `@correction:C-7`

```gherkin
Given a byte header that does not match a bzImage magic (x86_64) or a raw PE
  Image magic (aarch64) for the given HostArch
When KernelImage::validate is called
Then it returns a KernelFormatError naming the format, before any hypervisor
  process is spawned
And a genuinely valid x86_64 bzImage / aarch64 raw Image header validates
```

**Crafter notes**: The pure/impure split is load-bearing — the caller does
the `read`, the validator does not (Functional Core / Imperative Shell).
Also cover: distro `vmlinuz` on aarch64 (UKI wrapper) correctly fails
`validate` without unwrapping — unwrapping is BYO-artifact's job, not the
platform's, per `[D4]`.

#### S-VM-18: `memory.max` can never equal declared guest RAM

**Driving port**: `MemoryPlan::derive(declared)` (pure function, the ONLY constructor)
**Tags**: `@contract-shape:pure-function` `@property` `@ac-07` `@tier1` `@in-memory` `@mandatory:mutation_target` `@correction:C-3`

```gherkin
Given any declared byte count (resources.memory_bytes)
When MemoryPlan::derive(declared) constructs the plan
Then guest_bytes() always equals the declared figure exactly
And cgroup_max_bytes() is always strictly greater than guest_bytes()
And there is no MemoryPlan constructor that can make the two equal
```

**Crafter notes**: `@property` over `u64` — this is Hebert's "invariant"
pattern applied to a genuinely unbounded domain (any declared byte count).
Companion to S-VM-20 (`reserve_bytes` itself, a hard DELIVER-measured
dependency — see below). Complementary static enforcement — the
`xtask dst-lint` clause banning struct-literal construction of `MemoryPlan`
outside `overdrive-core` (`brief.md` §113) is a `xtask/src/dst_lint.rs` unit
test; see `distill/wave-decisions.md`'s dst-lint-clause decision.

#### S-VM-19: A VM that exceeds its declared memory is diagnosed as OOM, not a bare signal 9

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, cgroup memory pressure induced)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-07` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has deployed a VM workload whose guest workload is made to exceed
  the cgroup's memory.max
When the guest is cgroup-OOM-killed
Then "overdrive workload describe" reports TransitionReason::VmOutOfMemory
  with the observed limit and oom_kill_count
And the disposition is StoppedBy::Process (a crash), never
  StoppedBy::PlatformReclaimed
```

**Crafter notes**: D-3 fold-in (ADR-0082 §D8). `CgroupAccounting::oom_kill_count`
is read ONCE, post-mortem, immediately after the exit watcher's `wait()`/
`recv()` resolves and before any teardown — never a live subscription
(that stays deferred). Distinguish from `ExecDriver`'s own (still-deferred,
unreduced) OOM path — this closes the VM path only.

#### S-VM-20: `reserve_bytes` is a measured constant, never a guess

**Driving port**: `reserve_bytes(guest_bytes)` (pure function)
**Tags**: `@contract-shape:pure-function` `@property` `@ac-07` `@tier1` `@in-memory`

```gherkin
Given any guest_bytes value
When reserve_bytes(guest_bytes) is evaluated
Then the returned value is >= 0 and <= a documented upper bound (a fraction
  of guest_bytes plus a fixed floor, both named in the function's own
  docstring)
And the docstring cites the real-boot measurement (memory.current /
  memory.stat, never RSS) the bound was derived from
And the function is never a persisted field -- it is evaluated fresh at
  every MemoryPlan::derive call
```

**Crafter notes**: Ships as a `todo!("RED scaffold: …")` at DISTILL time —
this is a genuinely hard DELIVER dependency (intake precedent #7's "magic
version floor" failure with different units), not a routine scaffold. The
crafter measures before writing the body (handoff item "To
`nw-software-crafter`" #1). **`@mandatory:mutation_target` is deliberately
NOT on this scenario at DISTILL time** — a `todo!()` body has nothing to
mutate, and tagging it here would be a vacuously satisfiable gate (an
earlier draft of this catalogue made and then corrected this same mistake
once already). The mutation obligation is restated instead as a DELIVER-step
gate: `brief.md` §113's mutation table itself says *"`reserve_bytes` joins
this list at the DELIVER step that gives it a body, not at Slice 01"* — the
step that replaces the `todo!()` with a real measurement-derived body adds
`@mandatory:mutation_target` to this scenario in the SAME commit and runs
`cargo xtask mutants --file` against it before closing that step. The Then
above is restated as machine-checkable bounds (a range assertion, not "is
derived from a measurement") precisely so that mutation run has something to
kill once the body exists.

### AC-18: `VmDriver::stop` is total over every point in the start path (ADR-0082 §D4)

#### S-VM-76: `VmDriver::stop` handles pre-beacon stop, an unresponsive guest, an already-dead VMM and a double stop — none of them a crash

**Driving port**: `VmDriver::start` / `VmDriver::stop` — component-scope acceptance case against `SimVmm` (see the Driving Ports table row above; the enforcement vehicle ADR-0082 §D4 names by name)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-18` `@tier1` `@in-memory` `@example` `@mandatory:trait_contract`

**Crafter notes**: Per Mandate 9, a component-scope test at this layer is
example-only — four FIXED, hand-enumerated call sequences against `SimVmm`,
not generated input. `vmm_equivalence.rs` (S-VM-90) drives the `Vmm` port
only and structurally cannot reach `VmDriver::stop`'s guest half (the
`SHUTDOWN` byte lives on the beacon connection `VmDriver` holds, not on the
`Vmm` port) — this scenario is the enforcement ADR-0082 §D4 says would
otherwise be quietly shed by the D1-era relocation of `shutdown` out of the
port.

```gherkin
Given a VmDriver wired to a SimVmm, exercised through four sequences:

  (a) stop arrives BEFORE the guest has beaconed (between Vmm::create and
      accept_ready -- LiveVm.beacon is None)
  (b) stop arrives after the guest has beaconed, but the guest never reads
      the SHUTDOWN byte (an unresponsive guest)
  (c) stop arrives after the VMM process is already dead (Vmm::terminate
      observes VmTermination::Killed / an already-gone process)
  (d) stop is called twice in succession for the same allocation

When VmDriver::stop runs each sequence
Then (a) skips the beacon write entirely and goes straight to
  Vmm::terminate -- there is no connection to write SHUTDOWN to
And (b) escalates to Vmm::terminate once VM_SHUTDOWN_REQUEST_DEADLINE (2s)
  elapses on the unread write
And (c) returns Ok without erroring on an already-terminated process
  (idempotent terminate)
And (d) the second call is a no-op -- neither call panics, and the
  allocation reaches Terminated / Stopped { by: Operator } exactly once
And in ALL FOUR sequences the allocation's terminal disposition is
  Terminated / Stopped { by: Operator } -- NEVER a crash classification
```

---

## Cross-cutting — `VmReclamation` reconciler (SD-1, Bar 2)

Node-scoped, no single owning user story — needed the moment `[vm]`
allocations exist (Slice 01 onward), and its Tier-3 shapes depend on Slice
03's stop path existing to construct the failed-stop-orphan fixture. Per
ADR-0083 §D7 and brief §105a. Layer classification: `plan_reclamation` is a
**pure function** — layer 1/2 discipline (PBT full, `@property`) applies;
the two executors and the Tier-3 boot/tick shapes are layer 3+
(example-only, per Mandate 9/11).

### AC-08: The diff is pure, total, and safe by construction

#### S-VM-21: Mid-run drift repairs without a `serve` restart

**Driving port**: `overdrive serve` steady-state convergence loop (real, `VM_RECLAMATION_SWEEP_INTERVAL` advanced via `SimClock` in-memory, or real 30s wait in Tier-3)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-08` `@tier1` `@in-memory` `@property` — companion Tier-3 shape `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given a VM allocation is stopped and its cgroup scope removal is made to fail
When a later steady-state tick runs, WITHOUT restarting serve
Then the stranded scope, run directory and rootfs clone are all reclaimed
```

**Crafter notes**: THE AC that distinguishes Bar 2 from Bar 1 (brief
§105a.10 AC1) — under converge-on-boot this could only ever pass by
restarting the process. In-memory shape: `SimVmHostState` + `SimClock`
advanced past `VM_RECLAMATION_SWEEP_INTERVAL`. `plan_reclamation` itself is
exercised as a `@property` test separately (S-VM-31); this scenario proves
the WAKE mechanism (§105a.8) actually fires it.

#### S-VM-22: A live VMM whose allocation row is already terminal is killed at tick N

**Driving port**: `plan_reclamation` (in-memory) + Tier-3 shape through `overdrive serve`
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-08` `@tier1` `@in-memory` — Tier-3 companion `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given an allocation's row is Terminated (an unstoppable orphan -- the VMM
  process survived a failed stop)
When a reclamation tick runs
Then the surviving VMM process, its cgroup scope, run directory and rootfs
  clone are all gone after the sweep
```

#### S-VM-23: Boot-epoch reclamation settles before `adopt_on_restart_recovery` reads the tree

**Driving port**: `overdrive serve` boot sequence
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-08` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given N surviving VM cgroup scopes from a prior serve process
When "overdrive serve" reboots
Then the boot-epoch VmReclamation drive runs BEFORE
  adopt_on_restart_recovery, and every rmdir it issues has settled
  (succeeded or NotFound) before that pass reads the tree
And adopt_on_restart_recovery does NOT refuse the boot with
  NetnsRecoveryError::ObserveRead
And every reclaimed allocation's netns is treated as orphaned and reclaimed
```

#### S-VM-24: THE SAFETY HALF — a supervised, non-terminal VM survives every tick

**Driving port**: `plan_reclamation` (in-memory) + DST invariant `SupervisedVmSurvivesEveryTick`
**Tags**: `@contract-shape:unbounded-preservation` `@happy_path` `@ac-08` `@tier1` `@in-memory` `@property` `@mandatory:mutation_target`

```gherkin
Given a VM allocation is running and supervised (its driver reports the
  allocation in live_allocations())
When repeated reclamation ticks run
Then the VMM process, its cgroup scope, run directory, rootfs clone and
  observation row are ALL still present, unmodified, after every tick
```

**Crafter notes**: Without this AC the reconciler passes its entire suite
by killing everything (brief §105a.10 AC4 verbatim). This is the DST
invariant `SupervisedVmSurvivesEveryTick` (`assert_always!`) — mandatory,
not optional coverage.

#### S-VM-25: A terminal-row VMM with no live authorship claim — both shapes

**Driving port**: `plan_reclamation` (in-memory) + Tier-3
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-08` `@tier1` `@in-memory` `@property` — Tier-3 companion `@tier3` `@real-io` `@requires-kvm` `@mandatory:mutation_target`

```gherkin
Scenario shape (a) — the restart orphan:
Given a terminal allocation row survived a serve restart (no exit watcher
  exists for it at all)
When a reclamation tick kills the surviving VMM
Then the row is BYTE-UNCHANGED afterwards -- state, terminal, reason,
  updated_at, last_terminated, restart_count, and restart_counts[alloc_id]
  are all identical to before the tick

Scenario shape (b) — the failed-stop orphan:
Given an operator stop authored the terminal row and released the driver's
  claim, but the kill itself failed and the VMM (and its watcher) are still
  physically alive
When a reclamation tick's kill_scope wakes the surviving watcher
Then no ExitEvent reaches the observer for that allocation
And the row is BYTE-UNCHANGED afterwards, same fields as shape (a)
```

**Crafter notes**: Two distinct mutation targets. Shape (a) catches an
implementation that collapsed `DiscardStrandedArtifacts` into
`ReclaimAllocation` (still kills the VMM, still passes S-VM-22, betrays
itself only by re-classifying an honest ending). Shape (b) is the ONLY
route to catching a watcher that outlives its authored ending — an
implementation that does not gate `ExitEvent` emission on the atomic
`Held → EndingInFlight` transition (brief §105a.3 transition 3) emits an
event whose observer write advances `updated_at` at minimum, which is why
the assertion covers `updated_at`, not only the class-bearing fields.

#### S-VM-26: A reaped Job-kind VM is re-driven, never finalised with a fabricated exit code

**Driving port**: `WorkloadLifecycle::reconcile` (in-memory, exercising `is_natural_exit`'s new `&& !is_platform_reclaimed` clause)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-08` `@tier1` `@in-memory` `@property` `@mandatory:mutation_target`

```gherkin
Given a Job-kind VM allocation is reclaimed by the platform (its intent
  still stands -- the platform owes a replacement, per DD-1)
When the restart/backoff reconciler ticks
Then the allocation is re-driven (restarted), NEVER finalised via
  FinalizeFailed { terminal: Some(TerminalCondition::Failed { exit_code:
  Some(0) }) }
```

**Crafter notes**: DD-1's headline trap, and the Then's variant was
corrected — the earlier wording named `Terminated / Completed{exit_code:
0}`, but `brief.md` §104 and §105a.10 both pin the actual trap as the Job
finalise branch (`workload_lifecycle.rs:622-639`) fabricating
`TerminalCondition::Failed { exit_code: Some(0) }` on a workload that never
exited. As originally worded, the negative assertion would pass an
implementation that produces exactly the buggy variant the design names,
because `Failed{Some(0)}` is not `Completed{0}` — the same "AC that fails
against correct behaviour" shape (C-5) this catalogue corrects elsewhere.
The `is_natural_exit` clause is the ONLY predicate whose meaning changes
(`is_intentionally_stopped` / `is_restartable` need no change).

#### S-VM-27: Six consecutive `serve` restarts leave every VM running, never `RestartBudgetExhausted`

**Driving port**: `WorkloadLifecycle::reconcile` (in-memory, `RESTART_BACKOFF_CEILING = 5`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-08` `@tier1` `@in-memory` `@mandatory:mutation_target`

```gherkin
Given a VM allocation is reclaimed and restarted six times in a row (each
  driven by a simulated serve restart)
When the sixth reclamation-driven restart's budget check runs
Then the restart budget check at the backoff-ceiling branch (:679/:703)
  excludes Platform Reclamation from the attempts count
And the allocation is still restartable -- RestartBudgetExhausted is never
  reached from reclamation alone
```

**Crafter notes**: The ceiling branch was missed by an earlier design
draft's own three-line claim (caught by an emission-level property test,
not a site enumeration) — this scenario is that gap made concrete.

#### S-VM-28: Restart count and last-terminated populate together, in one scenario

**Driving port**: `overdrive workload describe` after a reclaim-and-restart cycle (Tier-3)
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-08` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given a VM allocation is reclaimed and then restarted
When Ana runs "overdrive workload describe"
Then restart_count has incremented by exactly one
AND last_terminated is populated with the reclamation disposition
  (StoppedBy::PlatformReclaimed)
```

**Crafter notes**: Deliberately one scenario, not two — asserting only the
budget passes an implementation that erased the occurrence; asserting only
the count passes one that consumed the budget silently. Per ADR-0078,
"a convergent record cannot answer 'did it happen'."

#### S-VM-29: The Service-path analogue — a reclaimed alloc is not handed a fabricated probe failure

**Driving port**: `ServiceLifecycle::reconcile` (in-memory, exercising the `startup_probe_failed_action` gate)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-08` `@tier1` `@in-memory` `@mandatory:mutation_target`

```gherkin
Given a Service-kind allocation (exec-driven — [vm]+[service] is rejected
  until #257) is reclaimed after reaching Running but before Stable
When the service lifecycle reconciler ticks
Then startup_probe_failed_action returns None for that allocation
And no ServiceFailed { StartupProbeFailed } is emitted for probes that
  never actually failed
```

**Crafter notes**: This is an exec-Service case TODAY (not VM), and a VM
case only once #257 lands — but the reclamation class is not VM-specific,
so it is covered now. `service_lifecycle.rs`'s liveness branch (`:769`) was
already safe; the other four emit sites were not, and this is the one that
was missed.

#### S-VM-30: A node that uninstalled cloud-hypervisor still reclaims its VM survivors

**Driving port**: `overdrive serve` boot (Tier-3, no `Vm` registry entry)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-08` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given a node has surviving VM cgroup scopes, run directories and clones from
  a prior boot, but cloud-hypervisor has since been uninstalled
When "overdrive serve" boots
Then the node boots (VmHostState composes unconditionally, unlike Vmm)
And the boot-epoch reclamation pass still reclaims every surviving VM
  allocation (supervision set reads Observed(∅), which is authorising, not
  a missing observation)
```

**Crafter notes**: Falsifiable form of "reclamation is NOT `Vmm`-gated"
(brief §104 row 5).

#### S-VM-31: `plan_reclamation` is pure and total over the decision table

**Driving port**: `plan_reclamation(desired, actual) -> Vec<Action>` (pure function)
**Tags**: `@contract-shape:pure-function` `@property` `@ac-08` `@tier1` `@in-memory` `@mandatory:mutation_target`

```gherkin
Given any (VmReclamationState, VmReclamationState) pair generated across the
  six rows of the decision table (non-terminal×authorised,
  non-terminal×held, terminal, unknown-on-VM-exclusive-surface×authorised,
  unknown-on-VM-exclusive-surface×held, cgroup-scope-only)
When plan_reclamation(desired, actual) is called
Then the emitted Vec<Action> matches the design's decision table exactly for
  every generated case
And the function performs no I/O and takes no port parameter (structural —
  "the observe pass wrote something" is not representable)
```

**Crafter notes**: `@property` — generate `VmReclamationState` pairs across
the six-row decision table (brief §105a.4). This IS the mandatory mutation
target the design names explicitly.

#### S-VM-32: Ending Class totality and disjointness

**Driving port**: proptest over `AllocStatusRow` terminal shapes
**Tags**: `@contract-shape:pure-function` `@property` `@ac-08` `@tier1` `@in-memory` `@mandatory:mutation_target`

```gherkin
Given any terminal AllocStatusRow the design's three Ending Classes could
  classify (Intentional Stop, Workload Failure, Platform Reclamation)
When the row is classified
Then it belongs to EXACTLY ONE of the three classes -- never zero, never two
```

**Crafter notes**: Named `P1` in the design (brief §105a.10) — a proptest,
not a compile-time guarantee (`EndingClass` enum was rejected as
disproportionate; brief §104 "The Ending Class surface").

### AC-19: The claim's lifecycle and the write-time guard are structural (iteration-2 review NEW-1 pins)

Five NEW-1 pins land in `brief.md` §105a.3/§105a.5/§105a.10. Two already had
scenarios (the release-on-deadline fragment inside S-VM-14/S-VM-25, and AC 5
shape (a)/(b) as S-VM-25). These five close the remaining gap.

#### S-VM-77: The claim releases on every `RetryOutcome` arm, not only `Wrote` -- the abandonment boundary

**Driving port**: the exit observer's loop body (`worker/exit_observer.rs:204-371`), component-scope acceptance case
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-19` `@tier1` `@in-memory` `@property` `@mandatory:mutation_target`

```gherkin
Given a VmDriver holding a Held claim on an allocation, exercised across the
  three RetryOutcome arms the observer's row write can resolve to (Wrote,
  Failed -- retry budget exhausted, NoPriorRow -- no row to write against)
When the exit observer's handling of that ExitEvent returns, for each arm
Then the claim is released (transitions to absent) on ALL THREE arms, not
  only Wrote
And a driver whose claim was never taken (start never reached step 0) is
  unaffected -- release is idempotent, an unknown id is a no-op
And release_supervision converges to absent under ANY interleaving of the
  observer's release and a shim terminal-row arm's release for the same
  allocation
```

**Crafter notes**: Two failure directions, both named in brief §105a.3, and
both must be killed. Release-only-on-`Wrote` leaves a `Failed`/`NoPriorRow`
allocation claimed forever -- SD-1's unstoppable-orphan failure reintroduced
by the very fix meant to close it. This is `P5` in `brief.md` §113 ("the
authorship claim is released on every path") -- a proptest over the
transition table (§105a.3) asserting convergence to absent from any
interleaving of transitions 3-6, catching release-only-on-`Wrote` AND an
unconditional watcher-drop release (NEW-1 by a slower route, closed by
transition 4 firing only from `Held`). Complementary static enforcement —
the `xtask dst-lint` clause requiring the observer's loop body to contain
EXACTLY ONE `release_supervision` call, sitting OUTSIDE `match outcome`
(`brief.md` §113), is a `xtask/src/dst_lint.rs` unit test, path-scoped to
`worker/exit_observer.rs`; see `distill/wave-decisions.md`'s
dst-lint-clause decision.

#### S-VM-78: The hydration read order is `observe()` first, supervision LAST -- a booting VM must never be killed by stale supervision

**Driving port**: `VmReclamationState::hydrate_actual` (in-memory), scheduled as two separate reads with a VM start interleaved between them
**Tags**: `@contract-shape:unbounded-preservation` `@error_path` `@ac-19` `@tier1` `@in-memory` `@mandatory:mutation_target`

```gherkin
Given a reclamation tick's hydrate_actual begins reading VmHostState::observe()
  at t1
And a new VM allocation starts and takes its supervision claim at
  t2, where t1 < t2
When hydrate_actual's supervision read runs LAST, at t3 > t2
Then the freshly-started allocation is present in the supervision set read
  at t3, so reclamation_authorised(alloc) is false for it
And no ReclaimAllocation is emitted for the arriving allocation
```

**Crafter notes**: Pinned as the OPPOSITE of the obvious "fail toward held"
order (supervision first) -- brief §105a.2's asymmetry argument: a
*departure*-stale error (supervision read first) lands on a terminal row and
is caught by the write-time guard (S-VM-79); an *arrival*-stale error
(supervision read first, on a VM that STARTS in the gap) lands on a
non-terminal, booting VM's row and NOTHING downstream can catch it -- a live
VM dies. Reading `observe()` first and supervision last closes that second,
uncaught direction. This scenario schedules the two reads as genuinely
separate steps (not a single atomic snapshot) with the start interleaved
between them, which is the only shape that can distinguish the two orderings.

#### S-VM-79: A write-time terminality guard refuses a race -- total no-op, never a degradation to disposal

**Driving port**: `execute_reclaim_allocation` (component-scope, `overdrive-control-plane`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-19` `@tier1` `@in-memory` `@mandatory:mutation_target`

```gherkin
Given plan_reclamation emits ReclaimAllocation { alloc_id } for a
  non-terminal row observed at diff time
And the row is written terminal (by an unrelated ending-authoring path)
  BEFORE execute_reclaim_allocation's own re-read resolves
When execute_reclaim_allocation's find_prior_alloc_row guard reads the row
Then the executor performs NO kill_scope and NO discard_artifacts and
  writes NO row
And it returns Ok(()) -- this is not an error
And a vm.reclamation.refused event is emitted carrying the alloc_id and the
  observed terminal state
And the row is byte-unchanged by this executor's refusal
```

**Crafter notes**: `execute_reclaim_allocation`'s existing `find_prior_alloc_row`
re-read is promoted from a `workload_id` lookup to a GUARD over the whole
executor (iteration-2 review NEW-1; brief §105a.5). The refusal must NOT
degrade to `DiscardStrandedArtifacts` -- that would smuggle back the exact
one-command-two-behaviours shape DD-5's two-`Action`-variant split refuses.
The next tick re-observes and, for a genuinely terminal row, correctly
re-decides `DiscardStrandedArtifacts` on its own account.

#### S-VM-80: `P2` as a property directly over `VmReclamation`, not only worked examples on the sibling reconcilers

**Driving port**: `plan_reclamation(desired, actual) -> Vec<Action>` (pure function, proptest)
**Tags**: `@contract-shape:bounded-change` `@ac-19` `@tier1` `@in-memory` `@property` `@mandatory:mutation_target`

```gherkin
Given any VmReclamationState pair (desired, actual) generated across the
  decision table (S-VM-31's six rows), where actual.host or actual.allocations
  contains at least one row that is ALREADY a Platform Reclamation
  (is_platform_reclaimed(row) is true for that alloc_id)
When plan_reclamation(desired, actual) computes the Vec<Action>
Then the output contains NO FinalizeFailed for that alloc_id
And the output contains NO StopAllocation { terminal: Some(_) } for that
  alloc_id
```

**Crafter notes**: `brief.md` §113 states P2 now ranges over THREE
reconcilers (`WorkloadLifecycle`, `ServiceLifecycle`, `VmReclamation`) and
names itself "the binding one" -- "P1 [totality] alone would have passed the
whole first draft." S-VM-26/S-VM-27/S-VM-29 already exercise this class of
defect as worked EXAMPLES on `WorkloadLifecycle`/`ServiceLifecycle`; this
scenario is the missing THIRD leg -- the property stated directly against
`VmReclamation`'s own diff, which is what stops reclamation authoring a
terminal claim on a row it has already reclaimed. Property-shaped per
Mandate 9 (layer 1-2, `@property`), not example-based, because the input
space (arbitrary `VmReclamationState` pairs) is exactly what S-VM-31's
generator already produces.

#### S-VM-81: Reclaiming an SVID-holding allocation drops its SVID -- the fourth evaluation

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process, mTLS-composed `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-19` `@tier3` `@real-io` `@requires-kvm` `@mandatory:mutation_target`

```gherkin
Given Ana has deployed a VM workload on an mTLS-composed "overdrive serve"
  and the allocation has been issued a workload SVID
And the allocation's row becomes an unstoppable orphan (its stop failed but
  the VMM survived) and a reclamation tick kills it
When execute_reclaim_allocation completes
Then a DropSvid action was submitted for that allocation, alongside the
  other three evaluations the exit observer submits per exit (workload_lifecycle,
  backend_discovery_bridge, service_lifecycle)
And the node no longer holds a live SVID for the reclaimed allocation's
  leaf private key
```

**Crafter notes**: `execute_reclaim_allocation` submits the SAME four
evaluations the exit observer submits per exit
(`worker/exit_observer.rs:234`, `:254`, `:295`, `:318-320`). Omitting the
fourth (`svid_lifecycle`) never fires `DropSvid`, and the node keeps the
dead allocation's leaf private key across every future `serve` restart that
reclaims a VM -- ADR-0067 O2 (leak-resistance) broken, every OTHER scenario
in this catalogue still green, because none of them asserts on SVID state.
This is the ONLY scenario in the catalogue that would catch that omission.

### AC-20: ESR — progress and stability specifications for `VmReclamation` (§105a.11)

Four `Invariant` variants, per `.claude/rules/testing.md` § "Tier 1 —
Deterministic Simulation Testing". Three are defined here; the fourth
(`SupervisedVmSurvivesEveryTick`) is already S-VM-24 above and is not
repeated.

#### S-VM-87: `VmReclamationIdempotentSteadyState` — a second reconcile over an unchanged observation is a no-op

**Driving port**: DST invariant `VmReclamationIdempotentSteadyState`, `cargo dst` + `Invariant` catalogue
**Tags**: `@contract-shape:unbounded-preservation` `@ac-20` `@tier1` `@in-memory` `@property` `@mandatory:esr_invariant`

```gherkin
Given a VmReclamationState observation that does not change between two
  consecutive ticks
When reconcile runs a second time over the identical (desired, actual) pair
Then the returned Vec<Action> is ALWAYS empty
```

**Crafter notes**: Stability class, `assert_always!`. Mirrors
`HydratorIdempotentSteadyState` (`invariants/mod.rs:360`) exactly. Falls
naturally out of `plan_reclamation`'s purity (S-VM-31) but is asserted as
its own DST invariant because the mandatory-ESR requirement is per-
reconciler, not inherited from a sibling.

#### S-VM-88: `VmReclamationConverges` — eventually no host state is attributable to a terminal or unknown allocation

**Driving port**: DST invariant `VmReclamationConverges`, `cargo dst` + `Invariant` catalogue
**Tags**: `@contract-shape:unbounded-preservation` `@ac-20` `@tier1` `@in-memory` `@property` `@mandatory:esr_invariant`

```gherkin
Given a DST run seeding an arbitrary mix of live, terminal and unknown VM
  allocations against SimVmHostState, with a SimClock advancing the 30s
  sweep cadence
When the harness runs to convergence
Then EVENTUALLY no host state on any surface (scope, run directory, clone)
  is attributable to a terminal or unknown allocation
```

**Crafter notes**: Liveness class, `assert_eventually!`. DST reachability is
`SimVmHostState` driven with a `SimClock` so `VM_RECLAMATION_SWEEP_INTERVAL`
is advanced deterministically rather than waited on wall-clock, per
`brief.md` §105a.11.

#### S-VM-89: `EndingInFlightIsNeverReclaimed` — the DST witness of the release-timing window `SupervisedVmSurvivesEveryTick` cannot reach

**Driving port**: DST invariant `EndingInFlightIsNeverReclaimed`, `cargo dst` + `Invariant` catalogue
**Tags**: `@contract-shape:bounded-change` `@ac-20` `@tier1` `@in-memory` `@property` `@mandatory:mutation_target` `@mandatory:esr_invariant`

```gherkin
Given a DST run in which an allocation's ending is IN FLIGHT -- its VMM has
  exited, or its stop has been issued, and its terminal row is not yet
  written
When a reclamation tick's plan_reclamation runs during that window
Then the in-flight allocation is ALWAYS absent from the ReclaimAllocation
  output
```

**Crafter notes**: The fourth invariant, added at iteration-2 review NEW-1
(brief §105a.11) and NOT foldable into `SupervisedVmSurvivesEveryTick`:
that invariant is scoped to membership in the OBSERVED supervision set, and
the exit window is precisely the interval in which a process-death handle
reading has just REMOVED the allocation from that set -- so the existing
invariant is vacuously satisfied exactly where the bug lives. This invariant
is stated over the WORLD (has this instance's ending been written yet?)
rather than over the set, which is the only framing that can witness the
window at all. It fails the moment an implementation reverts to releasing
the claim at process death, at `wait()`'s return, or at the watcher's
return -- exactly the defect S-VM-25 shape (b) exercises at the row level;
this is its DST-invariant-level counterpart.

---

## Slice 02 — `boot-failure-vocabulary`

Consumes: US-VM-2, US-VM-6, contradiction C-7 (kernel-format vocabulary), K3.

### AC-09: Every VM start failure surfaces a distinct, named, operator-actionable reason

#### S-VM-33: A missing kernel artifact is named precisely

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-09` `@tier3` `@real-io` `@kpi:K3`

```gherkin
Given "overdrive serve" booted while the configured VM kernel path contained a
  valid image, so the Vm driver composed successfully
And that exact kernel path is then DELETED after composition but before this
  allocation's VmDriver::start performs its per-allocation verification
When the platform attempts to start the allocation
Then the allocation is Failed with TransitionReason::VmKernelNotFound
  naming the exact path
And the reason is distinct from a missing rootfs
```

**Crafter notes**: This is the kernel counterpart to S-VM-35's already-ratified
TOCTOU shape. A path that is missing at `overdrive serve` boot cannot produce
the allocation-level `Failed` row asserted here because the validated
`KernelImage` / VM capability never composes. DWD-24 therefore preserves the
scenario's ID and expected operator result while making its producer reachable:
valid at composition, absent at this allocation's start. The per-allocation
reopen is specified by ADR-0082 §D2.4; it constructs typed
`VmStartFailure::KernelNotFound`, never a parsed string.

#### S-VM-34: A missing rootfs artifact is named precisely

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-09` `@tier3` `@real-io`

```gherkin
Given Ana has deployed a VM workload whose [vm] rootfs path does not exist
When the platform attempts to start the allocation
Then the allocation is Failed with TransitionReason::VmRootfsNotFound
  naming the exact path
And the reason does not collide with VmKernelNotFound
```

#### S-VM-35: The cloud-hypervisor binary vanishes between admission and start — a TOCTOU window S-VM-12's boot-time gate does not close

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, cloud-hypervisor present at boot, removed after admission)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-09` `@tier3` `@real-io` `@correction:contradiction-fix`

```gherkin
Given "overdrive serve" booted with cloud-hypervisor present -- the Vm
  driver is composed and a [vm] deploy is admitted
And the cloud-hypervisor binary is then REMOVED from the host, after
  admission but before this specific allocation's VmDriver::start actually
  spawns it
When the platform attempts to start the allocation
Then the allocation is Failed with TransitionReason::VmHypervisorAbsent
  naming the paths searched
And the reason is distinct from a missing kernel or rootfs artifact
```

**Crafter notes**: Rewritten to fix a contradiction the earlier draft did
not catch: SD-5's composition gate (ADR-0083 §D2/§D4) checks
`cloud-hypervisor`'s presence ONCE, at `overdrive serve` boot -- the same
precondition S-VM-35 originally described ("the host has no cloud-
hypervisor binary installed") makes the deploy REJECTED AT ADMISSION with
no allocation created at all (S-VM-12), which contradicts the original
Then ("the allocation is Failed") -- an allocation can never exist under
that precondition. `TransitionReason::VmHypervisorAbsent` is genuinely
reachable only through the narrower TOCTOU window this rewrite describes:
the binary was present at boot (so the driver composed and admission
succeeds), and is removed before an individual `VmDriver::start` spawns it.
No admission-time re-probe exists per deploy -- SD-5's gate is boot-scoped,
not deploy-scoped -- so this window is real, not contrived.

#### S-VM-36: A guest that hangs during boot reports a timeout, not a missing artifact

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-09` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has deployed a VM workload whose rootfs init hangs forever
When the boot deadline elapses
Then the allocation is Failed with TransitionReason::VmBootDeadlineExceeded
  naming the deadline in milliseconds and the captured console tail
And the allocation never passes through Running
```

#### S-VM-37: An unclassified hypervisor failure carries its verbatim cause, labelled unclassified

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, a hypervisor failure not covered by the named vocabulary)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-09` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given a VM start fails for a reason the platform has no named variant for
When Ana reads workload describe
Then the reason carries the verbatim hypervisor error text
And it is labelled as unclassified rather than presented as a known cause
```

**Crafter notes**: The kernel-format case (C-7, an unloadable kernel
reported by CH as a firmware size cap) is explicitly NOT this scenario — it
is caught pre-boot by `KernelImage::validate` (S-VM-17) and never reaches
this fallthrough. This scenario proves the fallthrough exists for the case
`KernelImage::validate` genuinely cannot anticipate. **The fallthrough
variant is `TransitionReason::DriverInternalError { detail }`**
(`transition_reason.rs`) — the EXISTING generic "driver returned an
uncategorised failure that did not fit any of the more specific cause
variants; falls back on the verbatim driver `Display` text in `detail`"
variant, not a new one this feature mints. DWD-24 supplies it through
`DriverStartClass::Unclassified { driver: DriverType::Vm }`; the conversion
copies the already-captured verbatim `DriverStartFailure.detail` into
`DriverInternalError { detail }`. No named variant or compatibility parser is
invented for this scenario.

#### S-VM-41: A kernel that exists but is the wrong format for this hypervisor is named precisely, not reported as a firmware size cap

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-09` `@tier3` `@real-io` `@correction:C-7`

```gherkin
Given "overdrive serve" booted on an aarch64 host while the configured VM
  kernel path contained a valid raw PE Image, so the Vm driver composed
And that same path is then atomically REPLACED after composition but before
  this allocation's VmDriver::start with the host distro's UKI-wrapped
  /boot/vmlinuz bytes, which cloud-hypervisor cannot load as a raw PE Image
When the platform attempts to start the allocation
Then the allocation is Failed with TransitionReason::VmKernelFormatUnsupported
  naming the exact configured path and arch "aarch64"
And the reported cause reads as a format problem -- never a firmware size
  cap and never "UefiTooBig"
And the reason is distinct from VmKernelNotFound (the path exists; only its
  format is wrong)
```

**Crafter notes**: This is the **classification-join** half of C-7,
companion to S-VM-17's pure-function half — do not duplicate S-VM-17 here.
S-VM-17 already proves `KernelImage::validate(path, arch, header)` is pure
and rejects the bad magic bytes before any hypervisor process is spawned
(ADR-0082 §D2.4), covering the identical aarch64 UKI-wrapper artifact at
the function boundary. This scenario proves the join one layer up: the per-allocation verifier
constructs `VmStartFailure::KernelFormatUnsupported { path, arch, detail }`
and the exhaustive `From<&DriverStartFailure>` conversion produces
`TransitionReason::VmKernelFormatUnsupported { path, arch, detail }`
(ADR-0083 §D5 row 5), without parsing validator or CH text; the
operator-visible cause the CLI renders
is honestly worded as a *format* problem — never CH's misleading
`VmBoot(UefiLoad(UefiTooBig))` framing, which is exactly the lie this
variant exists to prevent from reaching the operator (`slice-02.md`'s
correction block, `brief.md` §104). CH's own verbatim text, if it is ever
present at all, belongs only in the row's free-form `detail`, never in the
variant's meaning — assert on the rendered cause text, not on the enum
discriminant alone, per this AC's own "verified by reading `workload
describe` output, not by asserting on an enum" acceptance line. The
post-composition replacement is load-bearing: starting `serve` with the bad
image would fail before an allocation exists, which cannot satisfy this
scenario's `Failed`-row assertion (checkpoint `3222f030`, DWD-24).
**Gap-closure note**: this scenario was absent from the original 87 — the
row-5 Cause variant ADR-0083 §D5 pins for Slice 02
(`VmKernelFormatUnsupported`) had no test-scenarios.md entry at all, a gap
a fable review surfaced by cross-checking `deliver/roadmap.json`'s
Slice-02 step against the ADR's five-row table. Placement:
`crates/overdrive-cli/tests/integration/vm_boot_failure_vocabulary.rs`,
alongside S-VM-33…37 (DWD-14).

### AC-10: `[vm]` + `[service]` is rejected before scheduling; `[vm]` + `[job]` / `[schedule]` are accepted

#### S-VM-38: A VM service spec is rejected with the reason it cannot be served

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-10` `@tier3` `@real-io`

```gherkin
Given Ana has written a spec declaring both [service] and [vm]
When she runs "overdrive deploy web.toml"
Then the deploy is rejected before anything is scheduled
And the error names guest networking, guest probes and guest-stack mTLS as
  missing, citing GH #257 and GH #222
```

#### S-VM-39: A VM job spec is accepted

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-10` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has written a spec declaring both [job] and [vm]
When she runs "overdrive deploy render.toml"
Then the workload is accepted and scheduled
And its VM allocation reaches Running through the production VmDriver path
```

#### S-VM-40: A scheduled VM job is accepted

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-10` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has written a spec declaring [schedule] with a cron expression and [vm]
When she runs "overdrive deploy nightly.toml"
Then the workload is accepted and scheduled
And when its first firing becomes due, its VM allocation reaches Running through
  the production VmDriver path
```

### AC-21: The shipped binary reaches the VM driver, and a node that cannot says so

Added by DWD-25 (2026-08-17), which ruled the artifact contract
per-allocation and deleted the `ServerConfig.vm_artifacts` node-level seam.
Both scenarios below assert against a composition that has **no** artifact
seam to inject through — that absence is the point, and is what separates
them from S-VM-39 (which drives `run_with_dataplane_and_vm_artifacts`) and
from S-VM-12 (which asserts node-boots-and-rejects, not message quality).

#### S-VM-54: A VM job boots from the artifacts its own spec names, with no node-level artifact configuration

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve` composed WITHOUT any VM artifact seam)
**Tags**: `@contract-shape:bounded-change` `@walking_skeleton` `@happy_path` `@ac-21` `@tier3` `@real-io` `@requires-kvm` `@kpi:K4`

```gherkin
Given a kernel and two distinct ext4 rootfs images are staged on the host in
  separate directories, each guest identifiably reporting which image it booted
And "overdrive serve" is running with no kernel or rootfs configured anywhere
  in its arguments, environment or configuration files
And Ana has written two specs, each declaring [job] and [vm], naming the same
  kernel and a different one of the two rootfs images
When she runs "overdrive deploy" for each of them
Then both workloads are accepted and both VM allocations reach Running through
  the production VmDriver path
And each allocation booted from the rootfs image its own spec named
```

**Crafter notes**: The **structural** proof of DWD-25 is that reaching Running
at all is impossible unless the spec's paths were read — after 03-07 there is
no other source of a kernel path in the process. The two-spec clause is the
**regression** proof: it is the assertion a re-introduced node-level default
would fail, and a single-artifact test would pass vacuously. Compose `serve`
through the ungated production entrypoint; do NOT reach for
`run_with_dataplane_and_vm_artifacts` or `run_with_vm_artifacts` — 03-07
deletes both. **The two images MUST live in separate parent directories, or be
distinguishable from inside the guest.** `RootfsPlan::for_alloc` derives the
clone destination as `<master_dir>/.overdrive-vm-rootfs-<alloc>.img`, which
does **not** encode the master's filename — two masters in one directory
produce hypervisor argv that differs only by allocation id, so the assertion
would not discriminate which *image* was booted and the scenario would pass
vacuously against a node-level default. Assert through the observable run
directory / hypervisor argv, never by reading `VmHostLayout`. This scenario is
the in-tree companion to verification expectation
`E06-vm-job-deploy-reaches-running`, which asks the strictly harder question
(the shipped binary's own argv, out of process, default features) and is K4's
instrument; keep the two consistent — if this passes and E06 does not, the
difference is the in-process seam and is itself the finding.

#### S-VM-82: A node whose hypervisor capability is absent tells the operator what is absent

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-21` `@tier3` `@real-io`

```gherkin
Given "overdrive serve" started on a host where the VM capability probe does
  not pass, and its startup log recorded the specific reason
And Ana has written a spec declaring [job] and [vm]
When she runs "overdrive deploy render.toml" and then "overdrive workload describe"
Then the allocation is Failed and the reported cause names the absent VM
  capability and where the specific probe reason was recorded
And the cause does not present the node's own capability limit as an internal
  platform error
```

**Crafter notes**: This is a **message-actionability** scenario and nothing
else. It does not restate S-VM-12's assertions (the node boots; the deploy
does not run a VM) — those stand unchanged and must not be duplicated here.
Per DWD-25 the class stays `DriverStartClass::Unclassified { driver }` and the
conversion still lands on `DriverInternalError`; only `detail` changes, which
DWD-24 pins as free-form verbatim text that is never a classification input.
**Mint no `TransitionReason` variant and add no per-driver branch to the
action shim.** Assert on the operator-visible rendering, not on the enum
discriminant. The typed admission-time rejection remains DWD-23's scoped
follow-up and is explicitly out of scope here. Not `@requires-kvm`: the whole
point is a host that cannot boot a guest.

---

## Slice 03 — `stop-restart-and-vmm-death`

Consumes: US-VM-3, US-VM-4, US-VM-7, contradiction C-4, K5, K7 (items 1–3).

### AC-11: Exit classification never derives from the hypervisor's own exit status

#### S-VM-42: A guest kernel panic is a crash, never a clean completion

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, guest kernel-panics after boot)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-11` `@tier3` `@real-io` `@requires-kvm` `@kpi:K1` `@mandatory:mutation_target`

```gherkin
Given Ana has deployed a VM workload whose guest kernel panics after boot
When the hypervisor process exits with status 0
Then the allocation is Failed with TransitionReason::VmGuestExitUnreported
And the allocation is NOT Terminated with a completed condition
```

#### S-VM-43: A hypervisor killed by the host (OOM) is a crash

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, host OOM-kills the VMM)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-11` `@tier3` `@real-io` `@requires-kvm` `@kpi:K5`

```gherkin
Given Ana has deployed a VM workload and its hypervisor process is
  OOM-killed by the host
When the platform observes the hypervisor exit without an agent report
Then the allocation is Failed
And the restart/backoff behaviour matches a crashed process workload
  (same reconciler, same ceiling, same backoff curve)
```

#### S-VM-44: Only an agent-reported exit produces a completed terminal state

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-11` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has deployed a VM workload whose guest command exits 0 and reports it
When the VM shuts down
Then the allocation is Terminated with completed exit code 0
```

**Crafter notes**: The original Then's second clause ("this is the ONLY
path in the workspace to that state") is a workspace-negative claim no
port-observable assertion can make — it is a static property of the code
(how many call sites can produce `Terminated / Completed{exit_code: 0}`),
not something this scenario's fixture can observe. Moved here as a
mutation-annotation instruction instead: `[D3]`'s join (§105) is a
`@mandatory:mutation_target` per `brief.md` §113 precisely so a mutation
that opens a second path to this terminal state is caught by the suite as a
whole (S-VM-02, S-VM-42, S-VM-43 all assert the COMPLEMENT — a non-agent-
reported exit does NOT reach this state), not by a single scenario's Then.

#### S-VM-45: An operator stop is never counted as a crash

**Driving port**: `overdrive job stop` (direct CLI handler call, real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-11` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has a running VM workload
When she stops it with the operator stop verb
Then the allocation is Terminated as operator-stopped
And no restart budget is consumed
```

### AC-12: Stop and restart converge like any other workload

#### S-VM-46: Stopping a VM workload reaches the same terminal state as a process workload

**Driving port**: `overdrive job stop` (direct CLI handler call, real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-12` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has a running VM workload
When she runs the operator stop verb
Then the guest is asked to shut down gracefully over its open vsock connection
And the allocation reaches Terminated as operator-stopped
```

**Crafter notes**: Graceful shutdown rides the guest's ALREADY-OPEN vsock
connection (a `SHUTDOWN` byte), not CH's `vm.power-button` API (rejected —
no `acpid` on x86_64, PSCI not ACPI on aarch64; ADR-0082 "Graceful shutdown"
decision table). `VM_SHUTDOWN_REQUEST_DEADLINE = 2s` bounds the wait.

#### S-VM-47: An unresponsive guest is stopped within a bounded grace period, not classified as a crash

**Driving port**: `overdrive job stop` (direct CLI handler call, real in-process `overdrive serve`, guest ignores the shutdown byte)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-12` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has a running VM workload whose guest ignores shutdown requests
When she runs the operator stop verb
Then the allocation reaches Terminated as operator-stopped within the grace
  period
And it is NOT classified as a crash
```

#### S-VM-48: A restarted VM boots from a clean, unmodified rootfs copy

**Driving port**: `overdrive deploy` (direct CLI handler call, real in-process `overdrive serve`) + crash-induced restart
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-12` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given a VM workload crashed after modifying its rootfs
When the platform restarts the allocation under backoff
Then the new allocation boots from an unmodified copy of the operator's
  original artifact
And the operator's artifact file on the host is byte-unchanged
```

### AC-13: The hypervisor process is confined, or the workload does not run

#### S-VM-49: An untrusted VM workload runs with a bounded, non-root, Landlock-confined hypervisor

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, on a host that supports the required confinement)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-13` `@tier3` `@real-io` `@requires-kvm` `@kpi:K7`

```gherkin
Given Ana has deployed a VM workload on a host that supports the required
  confinement
When the allocation reaches Running
Then /proc/<vmm-pid>/status reports a non-zero real AND effective Uid and Gid
And /proc/<vmm-pid>/limits reports Max file size and Max open files strictly
  below the SAME fields on the overdrive serve process (/proc/<serve-pid>/limits)
And the hypervisor was launched under a Landlock ruleset naming that
  allocation's own kernel, rootfs copy and API socket by CH's auto-derived
  grants, PLUS a directory read-write grant on that allocation's own run
  directory (C-4 — the vsock socket CH does not auto-derive a rule for)
And the ruleset names nothing outside those grants
```

**Crafter notes**: Named-PID comparison, never `self` — under a Tier-3
harness `self` is the test process, not the server (US-VM-7 AC note). The
Then's ruleset enumeration was corrected — the original wording said the
ruleset names "only" the kernel, rootfs copy and API socket, omitting the
run-directory read-write grant that is C-4's entire content and that
S-VM-53 separately asserts exists. As originally worded this scenario would
have FAILED against the design's own correct behaviour (the same C-5-shaped
defect this catalogue fixes elsewhere: an AC written against an incomplete
model of the ruleset). The full grant set is CH's auto-derived rules for
`--kernel` / `--disk` / `--serial file=` / `--api-socket`, plus the ONE
directory grant `VmRunDir::landlock_rules()` derives for the vsock socket —
see S-VM-53 for the directory-exclusivity argument that makes the grant
derivable.

#### S-VM-50: The confinement ruleset follows the operator's declared artifact paths, never a hardcoded directory

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, rootfs outside the default artifact directory)
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-13` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana's rootfs lives at /home/ana/scratch/render.ext4, not the default
  artifact directory
When the allocation starts
Then the VM boots successfully
And the hypervisor can reach the declared kernel and rootfs and nothing else
```

**Crafter notes**: The falsifiable half of "the ruleset is derived, not
hardcoded" — a hardcoded ruleset FAILS this scenario.

#### S-VM-51: A host that cannot confine the hypervisor refuses the workload

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, `Vmm::probe` fault-injected via `SimVmm` for the confinement-unavailable case)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-13` `@tier3` `@real-io` `@mandatory:mutation_target`

```gherkin
Given Ana has deployed a VM workload on a host that cannot supply the
  required confinement (e.g. no --landlock support below the version floor)
When the platform attempts to start the allocation
Then the allocation is Failed with TransitionReason::VmConfinementUnavailable
  naming the unavailable ConfinementControl (Landlock / Seccomp / UidDrop /
  RlimitFsize / RlimitNofile / KvmAccess)
And the hypervisor is NEVER started unconfined
```

**Crafter notes**: Fail-closed, injected at the port boundary (system
constraint 1) — the whole Lima test envelope runs one kernel, so no
genuinely Landlock-less host exists in it (unlike the non-reflink class,
this one IS a fixed-kernel-shape capability, so the injection claim holds).
Mutation target: warn-and-continue must be killed. **Injection seam
RESOLVED by the concurrent DESIGN pass** — see S-VM-13's note: the same
`ServerConfig.vmm_override: Option<Arc<dyn overdrive_core::traits::vmm::Vmm>>`
seam (ADR-0083 §D8, `#[cfg(feature = "integration-tests")]`-gated,
port-trait-boundary substitution, `.probe()` still runs unconditionally)
substitutes `SimVmm` into this Tier-3 test's `overdrive serve` boot.

#### S-VM-52: Confinement adds no new operator surface

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:unbounded-preservation` `@happy_path` `@ac-13` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana already deploys VM jobs with "overdrive deploy <spec>"
When she deploys a workload on a host that supports the required confinement
Then no new flag, table or verb is required
And the terminal state and exit code she reads are unchanged
```

#### S-VM-53: The vsock socket's Landlock grant is a directory grant, scoped to nothing else

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`) + `VmConfig::landlock_rules()` (pure)
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-13` `@tier3` `@real-io` `@requires-kvm` `@correction:C-4`

```gherkin
Given Ana has deployed a VM workload
When the hypervisor is launched
Then the run directory holds nothing but this VM's own sockets and logs
And the Landlock ruleset grants read-write on that directory (CH does NOT
  auto-derive a rule for the vsock socket it binds itself, unlike --kernel /
  --disk / --serial file= / --api-socket)
```

**Crafter notes**: Spike P5 correction 1 — omitting this grant fails as
`CreateVsockBackend(UnixBind(EACCES))`, which never mentions Landlock. The
directory-exclusivity property (SD-2) is what makes the grant derivable
rather than a list a crafter must remember. Complementary static
enforcement — the `xtask dst-lint` `"--landlock-rules"` clause (`brief.md`
§113: Landlock rules never built outside `VmRunDir::landlock_grant`) is a
`xtask/src/dst_lint.rs` unit test; see `distill/wave-decisions.md`'s
dst-lint-clause decision.

---

## Slice 04 — `vm-writes-output-the-operator-can-read`

Consumes: US-VM-8, US-VM-9, contradiction C-6, K8, K9, K7 (volume-carrying case).

### AC-14: A declared volume is a real, byte-faithful host↔guest share

#### S-VM-55: A guest's write to a declared volume is readable, byte-identical, on the host

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-14` `@tier3` `@real-io` `@requires-kvm` `@kpi:K8`

```gherkin
Given Ana has deployed a VM job declaring a volume from a host directory to
  a guest path
And her guest command writes a file to that guest path
When the job reaches a terminal state
Then the file is present and readable in the host directory she named
And its contents are byte-identical to what the guest wrote
And the host-side read is done by ordinary filesystem access, not through
  any Overdrive API
```

#### S-VM-56: A read-only volume refuses guest writes, host-side, and defeats an uncooperative guest

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-14` `@tier3` `@real-io` `@requires-kvm` `@mandatory:mutation_target`

```gherkin
Given Ana has deployed a VM job declaring a read-only volume
When the guest attempts to write inside that volume
Then the write fails inside the guest
And the host directory is byte-unchanged
And the enforcement holds even for a guest that tries to remount the share
  read-write (host-side export, not a cooperative -o ro flag alone)
```

**Crafter notes**: The Tier-3 case MUST defeat a guest-side-only
implementation — a cooperative guest's failed write alone is insufficient
evidence, per US-VM-8 AC 2's explicit "must defeat a guest-side-only
implementation" clause.

#### S-VM-57: A VM job declaring no volume behaves exactly as before volumes existed

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:unbounded-preservation` `@happy_path` `@ac-14` `@tier3` `@real-io` `@requires-kvm` `@guardrail`

```gherkin
Given Ana has deployed a VM job with no volume declared
When the guest runs and exits
Then the allocation reaches the same terminal state and exit code as it did
  before volumes existed (Slice 01 regression guard)
And no storage daemon is started for that allocation
And --memory shared=on is NOT derived (private memory backing)
```

#### S-VM-58: A missing volume source directory is named precisely

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-14` `@tier3` `@real-io`

```gherkin
Given Ana has deployed a VM job whose declared volume source directory does
  not exist
When the platform attempts to start the allocation
Then the allocation is Failed with TransitionReason::VmVolumeSourceNotFound
  naming the volume source and the path
And the reason is distinct from a missing rootfs and from a missing storage
  daemon
```

#### S-VM-59: A missing storage daemon is distinguished from a missing directory

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, no virtiofsd installed)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-14` `@tier3` `@real-io`

```gherkin
Given the host has no virtiofs storage daemon installed
When Ana deploys a VM job declaring a volume whose source directory exists
Then the allocation is Failed with TransitionReason::VmStorageDaemonAbsent
  naming the paths searched
```

#### S-VM-60: A volume that cannot be mounted in the guest never reports a completed run — the composite-lie case

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, mount made to fail inside the guest)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-14` `@tier3` `@real-io` `@requires-kvm` `@mandatory:mutation_target`

```gherkin
Given Ana has deployed a VM job declaring a volume
And the guest cannot mount that volume (e.g. a BYO kernel without
  CONFIG_VIRTIO_FS)
When the platform starts the allocation
Then overdrive-init refuses to exec the operator's command
And the allocation is Failed with TransitionReason::VmGuestMountFailed
  naming the volume that could not be mounted
And the allocation NEVER reaches a completed terminal state
```

**Crafter notes**: Without this AC, the command writes into the discarded
per-launch rootfs copy at `target`, exits 0, and `workload describe`
reports `Terminated / Completed{exit_code: 0}` over an EMPTY host
directory — every individual signal truthful, the composite false. This is
the AC the whole `[D4]` amendment exists to defend.

#### S-VM-61: Adding a volume does not widen what the hypervisor can reach

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, volume-carrying allocation)
**Tags**: `@contract-shape:unbounded-preservation` `@edge_case` `@ac-14` `@tier3` `@real-io` `@requires-kvm` `@kpi:K7`

```gherkin
Given Ana has deployed a VM job declaring a volume
When the allocation reaches Running
Then the hypervisor's Landlock ruleset does NOT contain the volume's host
  source directory -- only the storage daemon reaches that data
And the rest of the [D7] posture (uid, rlimits, cgroup, netns) is unchanged
  from a volume-free VM job (verified on the SAME allocation this scenario
  already boots)
```

### AC-15: The operator surface under `[[vm.volume]]` is closed

#### S-VM-62: An unknown key under [[vm.volume]] is rejected by name

**Driving port**: `WorkloadSpecInput::from_toml_str()` (pure function — in-process TOML parse boundary, no subprocess, no `overdrive serve` needed)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-15` `@tier1` `@in-memory`

```gherkin
Given a spec's [[vm.volume]] table declares a key other than source, target
  or read_only (e.g. "cache")
When the operator submits the spec
Then the deploy is rejected, naming the unrecognised key
```

#### S-VM-63: `RLIMIT_FSIZE` accounts for a memfd-backed guest under `shared=on`

**Driving port**: `VmConfig::rlimit_fsize()` (pure function)
**Tags**: `@contract-shape:pure-function` `@property` `@ac-15` `@tier1` `@in-memory` `@mandatory:mutation_target` `@correction:C-6`

```gherkin
Given any (rootfs image size, guest RAM) pair
When rlimit_fsize() is evaluated
Then the result is always max(rootfs image size, guest RAM)
And this holds regardless of whether shared=on is in play for THIS
  particular VmConfig (encoded uniformly from Slice 01)
```

**Crafter notes**: `shared=on` backs guest RAM with a memfd, and a memfd is
a file for `RLIMIT_FSIZE` purposes — sizing off the rootfs alone kills
every volume-carrying VM with an opaque `SIGXFSZ` (spike P5 correction 3).
A Tier-3 companion scenario (real volume-carrying boot, no `SIGXFSZ`) is
folded into S-VM-55's happy path — no separate case needed.

### AC-16: The storage daemon's death is classified as honestly as the hypervisor's

#### S-VM-64: A completed VM job is never reported as crashed by its own storage daemon's clean shutdown

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-16` `@tier3` `@real-io` `@requires-kvm` `@kpi:K9` `@mandatory:mutation_target`

```gherkin
Given Ana has deployed a VM job declaring a volume whose guest command exits 0
When the job completes and the platform tears the allocation down
Then the allocation is Terminated with completed exit code 0
And the storage daemon's clean exit is observed in the allocation's
  event/audit trail WHILE CONTRIBUTING NOTHING to ExitKind
```

**Crafter notes**: The Tier-3 case must DISCRIMINATE, not merely observe a
green result — asserting only "Completed" is vacuous (a do-nothing
implementation that ignores the daemon also produces it). Assert the
daemon's exit was observed AND contributed nothing — the before/during-
teardown guard is the actual mutation site.

#### S-VM-65: A storage daemon that dies mid-run never produces a clean exit

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, virtiofsd killed while the guest runs)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@ac-16` `@tier3` `@real-io` `@requires-kvm` `@kpi:K9` `@mandatory:mutation_target`

```gherkin
Scenario shape (a) — the guest never resolves an outcome of its own:
Given Ana has deployed a running VM job declaring a volume
When the storage daemon dies while the guest is still running
Then the allocation is Failed with TransitionReason::VmStorageDaemonDied
  naming the socket
And the allocation is NOT Terminated with a completed condition
And the disposition is StoppedBy::Process -- NEVER StoppedBy::PlatformReclaimed

Scenario shape (b) — the guest self-reports a clean exit AFTER the daemon
  died, and must not be believed:
Given Ana's storage daemon has already died while her guest is still running
And her guest's own command then exits 0 and reports EXIT 0 over vsock
When the exit observer classifies the resulting ExitEvent
Then TransitionReason::VmStorageDaemonDied is still reported, overriding the
  guest-reported clean exit
And the allocation is NEVER Terminated with completed exit code 0
```

**Crafter notes**: RESOLVED by the concurrent DESIGN pass — ADR-0083 §D5
gained **row 14**, `TransitionReason::VmStorageDaemonDied { socket: String,
exit_code: Option<i32>, signal: Option<u8> }` (Slice 04), a **distinct**
variant, not a reuse of `VmGuestMountFailed` (row 10, which stays scoped to
the guest-reported start-time mount failure). The fact is carried on a new
additive `ExitEvent.storage_daemon_died` field (mirroring ADR-0082 §D8's
`oom` field), set by `VmDriver`'s own direct supervision of the sidecar it
spawns (`virtiofsd` sits outside the `Vmm` port entirely, system constraint
9 — the same shape `ExecDriver` already uses for its own workload process).
`exit_observer::handle_exit_event` gains a second additive precedence check
that runs **AHEAD of `ExitKind` entirely** — NOT nested inside the
`Crashed` arm the way row 13 (`VmOutOfMemory`) is. Scenario shape (b) exists
because that ordering is the whole point and must be able to fail if it is
wrong: per `[D4]`, `overdrive-init` execs the operator's command and waits
on it — it does not validate the operator's own I/O — so a guest whose
writes silently failed after its share died can still exit 0 and report
`EXIT 0` over the beacon, resolving `ExitKind::CleanExit`. A check nested
inside `Crashed` (row 13's shape) would let that `CleanExit` resolve first
and reproduce `VmGuestMountFailed`'s composite-lie defect (row 10) one
execution phase later — exactly the "job wrote 40 frames and the share
died... job reports success anyway" failure US-VM-9's Problem statement
names. Disposition is always `StoppedBy::Process` (an ordinary crash),
NEVER `StoppedBy::PlatformReclaimed` — DD-3's two-axis rule (Cause and
Disposition are orthogonal; a crash is a crash regardless of what caused
it).

#### S-VM-66: The guest does not boot before its share is ready

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-16` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has deployed a VM job declaring a volume
When the platform starts the allocation
Then the guest does not begin booting until the storage daemon's socket is
  ready to serve
And if the share never becomes ready, the allocation is Failed with
  TransitionReason::VmStorageSocketTimeout naming the socket and the wait
```

#### S-VM-67: The storage daemon's launch argument never carries a weaker sandbox than `--sandbox=namespace`

**Driving port**: the storage daemon's launch-argument rendering site (pure function; a value with private fields and exactly one rendering site, mirroring `DiskAttachment::to_disk_arg()`'s D2.1 shape verbatim — ADR-0082 §D2.1 / ADR-0083 §D8's closing amendment pin the enforcement TIER, not a concrete type name; the exact type is Slice 04's own, not yet designed, and DELIVER's Slice 04 RED phase names it per CLAUDE.md § "Implement to the design")
**Tags**: `@contract-shape:pure-function` `@property` `@error_path` `@ac-16` `@tier1` `@in-memory` `@mandatory:mutation_target` `@correction:D8d`

```gherkin
Given any host capability signal describing whether --sandbox=namespace can
  be supplied for the storage daemon (available or unavailable)
When the storage daemon's launch argument is rendered from that signal
Then an available signal always yields a launch carrying --sandbox=namespace
And an unavailable signal always yields a typed refusal naming the
  unavailable sandbox -- never a launch
And no input to this function ever yields a launch carrying
  --sandbox=chroot, and no input ever yields a launch with --sandbox
  omitted
```

**Crafter notes**: **RESOLVED 2026-08-11 by explicit user ruling** (recorded
in `distill/wave-decisions.md` DWD-13; the ruling itself, and the ADR
amendments it drove, are already landed in ADR-0082 §D2.1's cross-reference
amendment and ADR-0083 §D8's closing amendment — read both before touching
this scenario). The prior BLOCKED note offered two honest paths and chose
neither; **the user has now chosen path (b): assert the case below
`overdrive serve`, at the launch-argument construction layer — the same
enforcement tier ADR-0082 §D2.1 already uses for `image_type=raw`: private
fields, exactly one rendering site, a pure unit test on the rendered value.
No storage-daemon supervision port is minted by this feature** — path (a)
(a future Slice-04 port mirroring §D8's `Vmm` fault-injection shape) is
explicitly not taken here, and is never inherited from this ruling if a
later slice mints one on its own merits (process supervision, restart,
health).

**Why the argv layer, not the deploy-level symptom (measured, not
stylistic).** The spike measured `image_type=raw`'s own *absence*
surfacing **two layers away**, as `CreateVirtioFs`/vhost-user
`ConnectionRefused` on the `--fs` (virtiofs) modes only, on a VMM reboot
that had nothing to do with the actual cause (`spike/findings.md` lines
867-893, quoting the exact CH v53 error chain). Asserting where the value
is produced — the rendering site — rather than on a downstream symptom is
the pattern that already works here; ADR-0083 §D8's closing amendment
draws the same line explicitly. A Tier-3 scenario driving `overdrive
deploy` to observe the eventual `Failed` allocation would reproduce
exactly that trap: a real virtiofsd's failure to honour the flag, a
supervision bug in the sidecar, or a probe that never runs would all look
identical to "the argument was wrong" three layers downstream, and only
the argv-layer test isolates the actual mechanism this decision governs.

**The mutation site this scenario exists to kill** (feature-delta.md AC-16
Scenario 4 checklist, verbatim): *"No code path starts the daemon with a
weaker sandbox — mutation target: turning the fail-closed arm into
downgrade-and-continue must be killed."* The reference implementation's
unrecorded `namespace`→`chroot` drift (precedent warning #6) is exactly
what a mutant flipping the unavailable-signal branch from "typed refusal"
to "render `--sandbox=chroot` and continue" reproduces; this scenario's
totality assertion (both branches, no third output) is what kills it.
Complementary static enforcement — a `xtask dst-lint` clause banning a
second `--sandbox=` rendering site outside the one function, mirroring the
existing `"--disk"` clause (`brief.md` §113) — is a Slice 04 DELIVER
obligation once the type lands; `brief.md` §113's table is Slice-01-scoped
today and does not yet carry this row (DESIGN's, not this DISTILL pass's,
to add).

**THE BOUNDARY — stated so it cannot be read as more than it is (per
ADR-0083 §D8's own closing honesty).** This scenario's `Then` proves
**only** what argument the rendering function constructs: rendering
`--sandbox=namespace` at one site, with no second call site and no field
that could carry `chroot`, is verifiable purely. It does **NOT** prove:
(a) that a **running** `virtiofsd` actually enforces `--sandbox=namespace`
once spawned (the spike already verified this once, for the happy path —
`spike/findings.md` line 362, "the default and genuinely in effect... a
mount+net sandbox, not a full one" — that verification is not repeated per
scenario here); or (b) that the **platform** genuinely turns a host's real
incapacity into a `Failed` allocation end-to-end, rather than `virtiofsd`
degrading or failing to start silently underneath a correctly-rendered
argv. Both (a) and (b) remain a **Tier-3 property of Slice 04**, exercised
against a real supervised `virtiofsd` process when that slice is designed
and built — undischarged by this scenario, and undischarged by this
DISTILL pass. Do not read this scenario's `Then` as proof of the runtime
posture.

**No separate Tier-3 runtime-half scenario is added by this pass.**
Reasoned, not omitted: (i) no storage-daemon supervision port exists or is
minted by this feature (the negative decision above), so there is no
`Vmm`-style seam to inject an unavailable-capability fault through — the
`ServerConfig.vmm_override` seam that resolves S-VM-13/S-VM-51 does not
reach here (ADR-0083 §D8, "What the seam does NOT reach"); (ii) the
single-kernel Lima test envelope genuinely supports `--sandbox=namespace`
(`spike/findings.md` line 362), so there is no genuinely-lying host to
exercise the failure on without an injection seam; (iii) building either
now would be exactly the CLAUDE.md-forbidden "invent API surface past the
design" move both ADR-0083 §D8 and the user's ruling explicitly decline.
If Slice 04's own future DESIGN mints a storage-daemon supervision port on
its own merits, the resulting Tier-3 fail-closed scenario is that slice's
own DISTILL addition — not retrofitted onto this one.

**Placement** (supersedes the prior Tier-3 `overdrive-cli` placement):
`overdrive-core`, `tests/acceptance/vm_*.rs` (exact filename TBD — DELIVER's
Slice 04 RED phase pins it once the launch-argument value type exists,
following DWD-04's forward-reference discipline), default lane, no Lima
needed — a genuine tier change, not a relabeling, from the Tier-3
`overdrive-cli`/`@real-io` placement this scenario previously carried.

#### S-VM-68: Nothing is left behind after a volume-carrying VM ends, on any path

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, multiple terminal shapes)
**Tags**: `@contract-shape:unbounded-preservation` `@edge_case` `@ac-16` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has deployed a VM job declaring a volume
When the allocation reaches any terminal state -- including a failed start,
  a readiness timeout, or a sandbox refusal
Then no storage daemon process remains for that allocation
And no vhost-user socket remains on the host
```

---

## Slice 05 — `resources-size-the-vm`

Consumes: US-VM-5, K10.

### AC-17: Declared resources drive guest-observable vCPU count and memory size

#### S-VM-69: Declared CPU translates to guest vCPUs

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-17` `@tier3` `@real-io` `@requires-kvm` `@kpi:K10`

```gherkin
Given Ana has deployed a VM workload declaring 2000 cpu_milli
When the guest boots
Then the guest reports two online CPUs (observed FROM INSIDE the guest, not
  asserted against the constructed hypervisor config)
```

#### S-VM-70: A sub-core CPU request still yields a usable VM

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-17` `@tier3` `@real-io` `@requires-kvm`

```gherkin
Given Ana has deployed a VM workload declaring 250 cpu_milli
When the guest boots
Then the guest reports one online CPU (floor at 1 -- no fractional vCPU)
And the allocation reaches Running
```

#### S-VM-71: Declared memory is what the guest gets

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`)
**Tags**: `@contract-shape:bounded-change` `@happy_path` `@ac-17` `@tier3` `@real-io` `@requires-kvm` `@kpi:K10`

```gherkin
Given Ana has deployed a VM workload declaring 2147483648 memory_bytes
When the guest boots
Then the guest reports approximately 2 GiB of memory
And "overdrive workload describe" reports the same declared figure
```

#### S-VM-72: Sizing holds on both memory backings

**Driving port**: `overdrive deploy` (direct CLI handler call per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess", real in-process `overdrive serve`, parametrized)
**Tags**: `@contract-shape:bounded-change` `@edge_case` `@ac-17` `@tier3` `@real-io` `@requires-kvm` `@kpi:K10`

```gherkin
Given a VM workload declaring a fixed cpu_milli and memory_bytes
When it is deployed once with NO volume (private memory) and once WITH a
  volume (--memory shared=on)
Then the guest-observed vCPU count and memory size match the declared
  figures IDENTICALLY on both memory backings
```

**Crafter notes**: One parametrized Tier-3 case, not two separate ones —
the reason Slice 04 (volumes) is ordered before Slice 05 (resources).

#### S-VM-73: vCPU derivation rounds up and floors at one

**Driving port**: the vCPU-derivation pure function (name pinned by DELIVER's DESIGN reading — the derivation rule itself is DESIGN's, per US-VM-5 Technical Notes)
**Tags**: `@contract-shape:pure-function` `@property` `@ac-17` `@tier1` `@in-memory`

```gherkin
Given any cpu_milli value in u32 (including 0 and values not evenly
  divisible by 1000)
When the vCPU count is derived
Then the result is always max(1, round_up(cpu_milli / 1000))
And the result is never zero for any input
```

---

## Cross-cutting — Port contract enforcement (adapter equivalence + the kill-authorising predicate)

Per `.claude/rules/development.md` § "Trait definitions specify behavior, not
just signature" — the trait's docstring is the contract, the equivalence
test is the enforcement.

#### S-VM-90: `Vmm` adapter equivalence — `CloudHypervisorVmm` and `SimVmm` observe the same behaviour

**Driving port**: `vmm_equivalence.rs` (drives both adapters through the same call sequence)
**Tags**: `@contract-shape:bounded-change` `@example` `@tier3` `@real-io` `@requires-kvm` `@mandatory:trait_contract`

**Crafter notes**: Per Mandate 9, layer 3+ is example-only — this is a
FIXED, hand-enumerated call sequence (including the named edge cases), not
Hypothesis/proptest-generated input. `@example`, not `@property`, is the
correct tag despite the "equivalence" framing sounding property-shaped.

```gherkin
Given the same sequence of Vmm calls (probe, create, terminate — including
  the documented edge cases: create replaces a stale clone destination,
  create removes its clone if the spawn fails, config.netns == None is not
  an error, terminate on an already-dead VMM is Ok, probe is idempotent)
When the sequence is driven against CloudHypervisorVmm and, separately,
  against SimVmm
Then both adapters produce observably equivalent outcomes at every step
```

#### S-VM-91: `VmHostState` adapter equivalence — including the `kill_scope` settle postcondition

**Driving port**: `vm_host_state_equivalence.rs`
**Tags**: `@contract-shape:bounded-change` `@example` `@tier3` `@real-io` `@mandatory:trait_contract`

**Crafter notes**: Same Mandate 9 reasoning as S-VM-90 — `@example`, not
`@property`; a fixed call sequence at layer 3, not generated input.

```gherkin
Given the same sequence of VmHostState calls (probe, observe, kill_scope,
  discard_artifacts)
When driven against RealVmHostState and SimVmHostState
Then both adapters produce observably equivalent outcomes at every step
And kill_scope does not return until the scope's rmdir has succeeded or
  returned NotFound (the settle postcondition the boot-epoch drive depends
  on) -- asserted on BOTH adapters
```

#### S-VM-92: `SupervisionSet::reclamation_authorised` is the one kill-authorising predicate

**Driving port**: `SupervisionSet::reclamation_authorised(&self, alloc)` (pure function)
**Tags**: `@contract-shape:pure-function` `@property` `@tier1` `@in-memory` `@mandatory:mutation_target`

```gherkin
Given any SupervisionSet value (Unavailable, or Observed(arbitrary set)) and
  any AllocationId
When reclamation_authorised(alloc) is evaluated
Then Unavailable ALWAYS returns false -- never "unsupervised"
And Observed(held) returns true if and only if alloc is NOT a member of held
```

**Crafter notes**: `Unavailable` being the `Default` is the whole trick
(brief §105a.3) — a crafter who reads the wrong half of `State` gets "nothing
is authorised" rather than "nothing is supervised." Mutation target: this
predicate is consulted by every row of `plan_reclamation`'s decision table
that can reach a live VMM (S-VM-31).

#### S-VM-93: `CgroupAccounting` adapter equivalence — including the D8 probe fault table

**Driving port**: `cgroup_accounting_equivalence.rs`
**Tags**: `@contract-shape:bounded-change` `@example` `@tier3` `@real-io` `@mandatory:trait_contract`

**Crafter notes**: Same Mandate 9 reasoning as S-VM-90/91 — `@example`, not
`@property`; a fixed call sequence at layer 3. Closes a coverage gap: three
new port traits landed with this feature (`Vmm`, `VmHostState`,
`CgroupAccounting`), and only the first two got an equivalence test — this
was the missing third. Per `.claude/rules/development.md` § "Trait
definitions specify behavior, not just signature," the equivalence test IS
the enforcement of the trait's contract; without it "production and sim
observe the same behaviour" is a slogan for this one port.

```gherkin
Given the same sequence of CgroupAccounting calls (probe, oom_kill_count
  read-once)
When driven against RealCgroupAccounting and SimCgroupAccounting
Then both adapters produce observably equivalent outcomes at every step
And oom_kill_count is read exactly once per call -- no adapter caches or
  re-reads the value across calls (read-once semantics, ADR-0082 §D8)
And BOTH adapters are driven through the three probe fault classes ADR-0082
  §D8 enumerates: a failed read (Substrate), content that is not valid UTF-8
  (SubstrateCorrupt), and content with no parseable oom_kill line
  (MissingOomKillKey) -- each producing the matching typed
  CgroupAccountingProbeError variant on both adapters
```

#### S-VM-94: The per-launch `FICLONE` clone fails closed on a non-reflink target -- self-application of the boot probe's own rule

**Driving port**: `CloudHypervisorVmm::create()` (adapter-level, direct call — not through the full deploy path)
**Tags**: `@contract-shape:bounded-change` `@error_path` `@tier3` `@real-io` `@mandatory:mutation_target`

```gherkin
Given a VmConfig whose rootfs clone destination is a REAL non-reflink target
  (a tmpfs directory or a loopback-mounted ext4 image -- no SimVmm)
When CloudHypervisorVmm::create(&config) attempts the per-launch rootfs
  clone
Then create returns Err with the typed FICLONE errno (EOPNOTSUPP or EXDEV)
And no full-copy fallback occurs -- the clone destination is empty or
  absent afterwards, never a byte-for-byte copy of the master image
And no hypervisor process was spawned
```

**Crafter notes**: The boot probe (S-VM-13/S-VM-75) proves the SUBSTRATE is
reflink-capable once, at `overdrive serve` boot. `brief.md` §107's
"self-application (principle 13, recursively)" closes the gap a probe alone
leaves open: a remount, a package change, or a staging-path override after
boot can invalidate the boot-time probe's result, so the PER-LAUNCH clone
must independently enforce the same rule — via the `FICLONE` ioctl directly
(never `cp --reflink=auto`, which silently degrades to a full copy on
`EOPNOTSUPP`/`EXDEV` with no error, per P4's measured 3.970s / +4096 MiB
cost). Without this scenario, an adapter implemented with
`cp --reflink=auto` stays green on Lima's reflink-capable filesystem
forever and only degrades in production. Placement:
`crates/overdrive-host/tests/integration/vmm_ficlone_per_launch.rs`, real
non-reflink fixture, gated `integration-tests`.

---

## `@real-io` Adapter Coverage Table

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `Vmm` (`CloudHypervisorVmm`) | YES | S-VM-01, S-VM-02, S-VM-14, S-VM-15, S-VM-94 (per-launch `FICLONE`), plus `vmm_equivalence.rs` (S-VM-90) |
| `Vmm` (`SimVmm`) | YES (fault injection, port boundary — `ServerConfig.vmm_override`, ADR-0083 §D8; capability-flag classes only, see S-VM-13's crafter note) | S-VM-13, S-VM-51 |
| `Vmm`, non-reflink substrate | YES (real substrate, no injection) | S-VM-75 |
| `VmDriver::stop` (component-scope, `SimVmm`) | YES | S-VM-76 |
| `VmHostState` (`RealVmHostState`) | YES | S-VM-21…30 Tier-3 shapes, plus `vm_host_state_equivalence.rs` (S-VM-91) |
| `CgroupAccounting` (real cgroupfs `memory.events` read) | YES | S-VM-19, plus `cgroup_accounting_equivalence.rs` (S-VM-93) |
| `overdrive-init` (guest PID 1, real vsock beacon + real exec/exit framing) | YES | S-VM-01, S-VM-02, S-VM-14, S-VM-15, S-VM-60 |
| `DriverRegistry` composition (discover → probe → insert) | YES | S-VM-11, S-VM-12, S-VM-13, S-VM-35, S-VM-75 |
| `MtlsInterceptWorker` gating (§D2a(c)) | YES | S-VM-05 (fail-closed arm), S-VM-74 (gated-off arm) |
| Spec parser — `[vm]` / `[[vm.volume]]` driver-table dispatch | YES (in-process; real deserializer, no subprocess needed for rejection-only cases) | S-VM-06, S-VM-07, S-VM-62 |
| `JobEnvelope` V1→V2 (rkyv, real serialize/deserialize) | YES | S-VM-10 |
| virtiofsd storage daemon (real supervised host process) | YES | S-VM-55, S-VM-56, S-VM-59, S-VM-64, S-VM-65, S-VM-66, S-VM-68 (excludes S-VM-67 — moved to `@tier1`/`@in-memory` pure launch-argument construction, DWD-13; the running-daemon half of `[D8d]` stays a Tier-3 property of Slice 04, undischarged by any scenario in this catalogue — see S-VM-67's own crafter note) |
| `VmReclamation` reconciler, real `overdrive serve` convergence loop | YES | S-VM-21 (Tier-3 companion), S-VM-22 (Tier-3 companion), S-VM-23, S-VM-28, S-VM-30, S-VM-81 (`svid_lifecycle` evaluation) |
| DST harness, `VmReclamation` ESR invariants | YES (Tier 1, `SimVmHostState` + `SimClock`) | S-VM-24, S-VM-87, S-VM-88, S-VM-89 |

Zero "NO — MISSING" rows.

---

## Driving Adapter Coverage (Mandatory, RCA fix P1)

Every CLI entry point / production driving surface DESIGN names, mapped to at least one direct-CLI-handler-driven scenario against a real in-process `overdrive serve` (per this project's no-subprocess convention, DWD-07):

| Driving surface | Protocol exercised | Covered by |
|---|---|---|
| `overdrive deploy <spec.toml>` | direct CLI handler call, real in-process serve | S-VM-01 (and the great majority of Tier-3 scenarios above) |
| `overdrive workload describe <id>` | direct CLI handler call, real in-process serve (read side) | Every Tier-3 scenario's `Then` clause |
| `overdrive job stop <id>` | direct CLI handler call, real in-process serve | S-VM-45, S-VM-46, S-VM-47 |
| `overdrive serve` (boot-time composition gate) | real process boot | S-VM-11, S-VM-12, S-VM-13, S-VM-23, S-VM-30, S-VM-75 |

No pipeline/service-level test (calling `VmDriver::start` directly) substitutes for any of the above — every row is a direct CLI handler call against a REAL in-process `overdrive serve` (never a hand-assembled test harness standing in for the composition root), per system constraint 1 and per this project's own `CLAUDE.md` "Build vertical slices through production entry points." The ONE deliberate carve-out is S-VM-76 (`VmDriver::stop`'s edge cases against `SimVmm`), justified in the Driving Ports table above by ADR-0082 §D4's own named enforcement vehicle — `vmm_equivalence.rs` structurally cannot reach the guest half of `stop`, so a component-scope test is the only way to exercise it at all.

---

## KPI Traceability

| KPI | Scenario(s) | What it exercises |
|---|---|---|
| K1 (exit-status fidelity, north star) | S-VM-01, S-VM-02, S-VM-42 | Guest exit code, not the VMM's, reaches `workload describe` on every terminal shape |
| K2 (Running honesty guardrail) | S-VM-03 | A guest that never runs init never reaches `Running` |
| K3 (≥4 distinct start-failure diagnoses) | S-VM-33…37, S-VM-41 | 5 distinct `TransitionReason` variants at Slice 02 alone (13 across the feature) |
| K4 (production path reachability, pass/fail bar) | S-VM-01, S-VM-12 | Real `serve` + `deploy` reaches the VM driver with no test-only wiring |
| K5 (crash-restart parity) | S-VM-43 | Same reconciler, ceiling, backoff as a crashed process |
| K6 (bounded time-to-Running) | S-VM-01 (companion timing assertion) | p50 ≤3s / p99 ≤10s over ≥20 deploys |
| K7 (confinement, guardrail) | S-VM-05, S-VM-49, S-VM-61 | 100% confined, 0 degraded-silently, 0 widened by a volume |
| K8 (output fidelity) | S-VM-55 | Byte-identical guest write reaches the host directory |
| K9 (storage-daemon death, guardrail) | S-VM-64, S-VM-65 | 0 false-crash, 0 false-clean, both directions |
| K10 (declared-size fidelity) | S-VM-69, S-VM-71, S-VM-72 | Guest-observed vCPU/memory match declared figure, both memory backings |

All 10 KPIs traced. K6 is the only "secondary" KPI (a companion assertion on an existing scenario rather than its own).

---

## AC-to-Scenario Traceability

| Story | AC(s) | Scenario(s) | Coverage |
|---|---|---|---|
| US-VM-1 | AC-01…AC-07, AC-18 (+ cross-cutting AC-08/AC-19/AC-20) | S-VM-01…20, S-VM-74, S-VM-76 | 5 UAT scenarios + 15 AC-derived + engineering constraints + the `MtlsInterceptWorker` gating case + `VmDriver::stop` totality |
| US-VM-2 | AC-09 | S-VM-33…37, S-VM-41 | 4 UAT scenarios + 2 additional (rootfs-not-found symmetry; S-VM-41 closing the ADR-0083 §D5 row-5 `VmKernelFormatUnsupported` gap found by fable review, DWD-14); S-VM-35 rewritten to the TOCTOU window it can actually reach (`wave-decisions.md` DWD-11) |
| US-VM-3 | AC-11 | S-VM-42…45 | 4/4 UAT scenarios |
| US-VM-4 | AC-12 | S-VM-46…48 | 3/3 UAT scenarios |
| US-VM-5 | AC-17 | S-VM-69…73 | 3 UAT scenarios + parametrized sizing + property |
| US-VM-6 | AC-10 | S-VM-38…40 | 3/3 UAT scenarios |
| US-VM-7 | AC-13 | S-VM-49…53 | 4 UAT scenarios + Landlock-directory-grant correction; S-VM-49's ruleset enumeration corrected to match S-VM-53 |
| US-VM-8 | AC-14, AC-15 | S-VM-55…63 | 7 UAT scenarios + engineering-constraint scenario |
| US-VM-9 | AC-16 | S-VM-64…68 | 5/5 UAT scenarios; S-VM-65's hedged `TransitionReason` variant RESOLVED by the concurrent DESIGN pass (ADR-0083 §D5 row 14, `VmStorageDaemonDied`); S-VM-67 RESOLVED by explicit user ruling (DWD-13) — rewritten to the pure launch-argument-construction layer, `@tier1`/`@in-memory`, no storage-daemon supervision port minted; the deploy-level fail-closed claim stays an undischarged Tier-3 property of Slice 04, not covered by any scenario in this catalogue |
| SD-1 (Bar 2 reconciler) | AC-08, AC-19, AC-20 | S-VM-21…32, S-VM-77…81, S-VM-87…89 | 5 ACs (105a.10) + 2 property tests (P1 totality, `plan_reclamation` purity) + DD-1 trap scenarios (a)/(b)/(d) + the five NEW-1 pins (abandonment boundary, hydration read order, write-time terminality guard, P2-over-`VmReclamation`, the fourth `svid_lifecycle` evaluation) + all four §105a.11 ESR invariants |
| Port contract enforcement (cross-cutting, no single owning story) | — | S-VM-90…94 | `Vmm`, `VmHostState`, `CgroupAccounting` adapter equivalence; `SupervisionSet::reclamation_authorised` purity; per-launch `FICLONE` self-application |

All 9 stories + the cross-cutting reconciler + the cross-cutting port-contract
scenarios covered. Zero AC without at least one scenario; zero UAT scenario
from the feature-delta.md without a matching test-scenarios.md entry.

---

## Error / Edge Path Coverage

Counts below are computed mechanically from the `**Tags**:` line of every
scenario (**88 total** — up from 74 after the adversarial-review
remediation pass added 13 new scenarios: the 3 missing §105a.11 ESR
invariants that were BLOCKER-cited but never defined, the 5 NEW-1-pin
scenarios, the 4th `svid_lifecycle` evaluation, the `CgroupAccounting`
equivalence test, the per-launch `FICLONE` self-application test, the
`VmDriver::stop` totality case, and the `MtlsInterceptWorker`-gating case;
plus a like-for-like TOCTOU rewrite of S-VM-35, which added no new ID);
then **87 → 88** (DWD-14, S-VM-41, closing the ADR-0083 §D5 row-5
`VmKernelFormatUnsupported` gap a fable review surfaced against
`deliver/roadmap.json`'s Slice-02 step).
Re-verified by `grep -c '^\*\*Tags\*\*:'` and cross-checked against
`grep -c '^#### S-VM-'` (both 88) after every edit in this pass, most
recently after S-VM-41's addition (DWD-14) — `@error_path` moves
**40 → 41** and `@contract-shape:bounded-change` moves **65 → 66**
(S-VM-41 carries both, mirroring S-VM-33…37's shape); `@property` stays
**21** (S-VM-41 is Tier-3/example-shaped, not a property scenario, same as
its AC-09 siblings). Mechanical recount also surfaced a pre-existing
off-by-one in the Self-Review Checklist's `@contract-shape:pure-function`
/ `@contract-shape:bounded-change` split (claimed 11/66, both were
already 12/65 before this pass — corrected in Self-Review Checklist item
13, below).

| Category | Count |
|---|---|
| `@happy_path` | 18 |
| `@error_path` | 41 |
| `@edge_case` | 12 |
| No happy/error/edge tag (pure `@property`/`@example` scenarios: S-VM-08, 10, 16, 18, 20, 31, 32, 63, 73, 80, 87, 88, 89, 90, 91, 92, 93) | 17 |
| `@property` | 21 |
| `@example` (fixed call sequences at layer 3, per Mandate 9 — S-VM-76, 90, 91, 93) | 4 |
| **Total distinct scenarios** | **88** |

Error + edge coverage: **53 of 88 ≈ 60%** — well above the 40% target,
consistent with this feature being fundamentally about honest failure
classification (K1/K2/K3/K7/K9 are ALL guardrail/diagnosis KPIs), and the
remediation pass's additions skewed further toward `@error_path` (the
NEW-1 pins, the fourth evaluation, and both adapter-equivalence-gap fixes
are all failure/edge-shaped by nature). S-VM-41 (DWD-14) keeps the ratio
unchanged at ≈60% — one more `@error_path` scenario over one more total
scenario. Unchanged by S-VM-67's rewrite (DWD-13) — it keeps its
`@error_path` tag; only its tier/mechanism tags moved (`@tier3 @real-io`
→ `@tier1 @in-memory`).

---

## Self-Review Checklist

- [x] 1. Walking skeleton declared: S-VM-01, tagged `@walking_skeleton @driving_port`, driven through real `overdrive serve` + `overdrive deploy` with a real Cloud Hypervisor VMM and a real guest kernel. Exactly one.
- [x] 2. Scenarios tagged `@real-io` / `@in-memory` / `@tier1` / `@tier3` per this project's four-tier model (mapped from the generic skill's Strategy A/B/C/D, which does not apply to this Rust workspace)
- [x] 3. Every driven adapter has at least one `@real-io` scenario — Adapter Coverage Table: 0 MISSING. Includes `CgroupAccounting` (`S-VM-93`, added in remediation — the earlier pass got equivalence tests for `Vmm` and `VmHostState` but missed the third new port trait)
- [x] 4. `SimVmm` fault-injection scope documented, corrected: it injects substrate LIES that CANNOT be produced on the single Lima kernel in the test envelope — capability-flag absence (Landlock flag/LSM, KVM access) and an unbindable run root — via `ServerConfig.vmm_override` (ADR-0083 §D8). **The non-reflink class is NOT in this set** — a tmpfs/loopback-ext4 staging directory IS a real, non-injected non-reflink substrate reachable in the same Lima harness, and is exercised for real at S-VM-75 (boot probe) and S-VM-94 (per-launch clone). **The sandbox-unavailable class (S-VM-67) was never in this set** — ADR-0083 §D8 explicitly rules it outside `vmm_override`'s reach (`virtiofsd` sits behind no `Vmm` method, and no other port exists for it). **RESOLVED, not by this seam**: explicit user ruling (DWD-13) moves S-VM-67 to the pure launch-argument-construction layer instead — no `SimVmm`/port injection of any kind, `@tier1`/`@in-memory`, no storage-daemon supervision port minted by this feature; see S-VM-67's own crafter note for the full boundary statement. Never a production effect the test hand-installs (system constraint 1 compliance, GH #248/ADR-0074 trap deliberately reproduced and closed in S-VM-05/S-VM-74)
- [x] 5. No container preference — all Tier-3 execution is `cargo xtask lima run --`, per project convention
- [x] 6. Mandate 7 scaffolding — see `distill/wave-decisions.md` DWD-06 for the scoped scaffold decision (this feature genuinely introduces new crates/modules, unlike the brownfield precedent that deferred scaffolding entirely). Corrected in this remediation pass: the accounting was internally inconsistent (claimed 15 scaffolds "for every scenario in Slice 01" when only 12 of 20 are scaffolded) and the deferred-scaffold list omitted S-VM-11/S-VM-12 — both fixed in `wave-decisions.md`. No new scaffolds were authored by this remediation pass; every new/modified scenario joins the existing deferred-to-DELIVER set with its crate/file destination recorded in DWD-04
- [x] 7. Driving Adapter coverage — `overdrive deploy` / `overdrive workload describe` / `overdrive job stop` / `overdrive serve` boot, all exercised via direct CLI handler call against a real in-process `overdrive serve` (never a subprocess, per `crates/overdrive-cli/CLAUDE.md`); see § Driving Adapter Coverage. One documented, justified carve-out: S-VM-76 (`VmDriver::stop` totality) is a component-scope acceptance case against `SimVmm`, named as its own enforcement vehicle by ADR-0082 §D4 because `vmm_equivalence.rs` cannot reach the relocated guest half of `stop`
- [x] 8. Error path coverage ≥ 40% — 60% (see § Error / Edge Path Coverage)
- [x] 13. **Contract Shape Classification (mandate 14, 2026-05-15)** — every one of the 88 scenarios carries a `@contract-shape:<pure-function|bounded-change|unbounded-preservation>` tag, mechanically recounted while adding S-VM-41 (DWD-14) via `grep '^\*\*Tags\*\*:' | grep -c '@contract-shape:<kind>'`: 12 `pure-function` (the `VmConfig`/`plan_reclamation`/`SupervisionSet` pure-function scenarios + the `JobEnvelope` V1 roundtrip — unchanged; **corrects a pre-existing off-by-one in this line, which read 11** — the true count was already 12 before this pass, confirmed by direct listing), 10 `unbounded-preservation` (S-VM-14, 24, 52, 57, 61, 68 — unchanged from the original set — plus S-VM-74, 78, 87, 88, added by the remediation pass: "no intercept state exists," "the freshest-read supervision set is never stale toward a booting VM," and the two ESR invariants whose statements are themselves open-ended non-enumerable claims), 66 `bounded-change` (a specific, nameable resource/row/field transition with a closed complement — S-VM-41 joins this class, DWD-14; 65 before this pass, not 66 as the prior text implied — the prior 11/66 split summed correctly to 87 by coincidence, masking the same off-by-one). 12 + 10 + 66 = 88. Zero untagged.
- [x] 9. Wave-decision reconciliation — 0 contradictions (see `distill/wave-decisions.md`)
- [x] 10. AC-to-scenario traceability complete — all 9 stories + the cross-cutting reconciler + the cross-cutting port-contract scenarios covered
- [x] 11. KPI traceability documented — all 10 KPIs (K1–K10) traced
- [x] 12. Property tests specified for every mandatory mutation-target pure function named in the DESIGN handoff (`MemoryPlan::derive`, `KernelImage::validate`, `DiskAttachment::to_disk_arg`, `VmConfinement::seccomp_arg`, `VmConfig::rlimit_fsize`, `plan_reclamation`, `SupervisionSet::reclamation_authorised`, Ending Class totality), plus P2 now stated directly as a property over `VmReclamation` (S-VM-80), not only as worked examples on the two sibling reconcilers
- [x] 14. **Adversarial-review remediation (2026-08-11)** — see `distill/wave-decisions.md` DWD-11 for the full finding-by-finding disposition. Three BLOCKER-cited-but-undefined ESR invariant scenarios defined (S-VM-87/88/89); zero dangling `S-VM-N` references remain anywhere in `test-scenarios.md`, `feature-delta.md`, or `wave-decisions.md` (mechanically re-verified, see DWD-11)
- [x] 15. **AC-09 completeness gap closed (2026-08-11) — S-VM-41, DWD-14.** ADR-0083 §D5 pins **five** Slice-02 Cause variants (row 5 is `VmKernelFormatUnsupported`); `deliver/roadmap.json`'s Slice-02 step 03-01 criteria enumerated only four, and the original 87-scenario catalogue had no entry for row 5 at all — a fable reviewer caught the narrowing by cross-checking the roadmap against the ADR. S-VM-41 added, cross-referencing S-VM-17 (the pure-function half already proven) rather than duplicating it. Every count in this file re-verified mechanically after the addition (see § Error / Edge Path Coverage).
