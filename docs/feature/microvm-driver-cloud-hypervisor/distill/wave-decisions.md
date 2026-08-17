# DISTILL Wave Decisions — microvm-driver-cloud-hypervisor

**Wave**: DISTILL (acceptance-designer)
**Owner**: Quinn (nw-acceptance-designer)
**Date**: 2026-08-11
**Status**: COMPLETE — handoff-ready for DELIVER (software-crafter), pending peer review by Sentinel (`nw-acceptance-designer-reviewer`).

---

## Reading Confirmation Checklist

| # | Artifact | Status | Path |
|---|---|---|---|
| 1 | Journeys | - not applicable | no `docs/product/journeys/{vm}.yaml` exists; DISCUSS derived the persona/JTBD directly (see feature-delta.md § Persona / § JTBD) |
| 2 | Architecture Brief | + read | `docs/product/architecture/brief.md` §§ 89–114 (System / Domain / Application Architecture for this feature) |
| 3 | KPI Contracts | - not found | `docs/product/kpi-contracts.yaml` does not exist; KPI targets taken from feature-delta.md § Outcome KPIs (K1–K10) |
| 4 | DISCUSS (feature-delta.md, compact form) | + read, in full | `docs/feature/microvm-driver-cloud-hypervisor/feature-delta.md` lines 1–2652 (Grounding G1–G6, Decisions D1–D8, Scope assessment + 3 re-assessments, Story map, System constraints, 9 user stories, KPIs, Risks, DoR, Handoff, Blockers) |
| 5 | SPIKE findings + wave-decisions | + read | `spike/findings.md` (summary read; full 3053-line evidence log consulted via `spike/wave-decisions.md`'s verdict table), `spike/wave-decisions.md` (PROMOTE, revised 2026-08-10) |
| 5b | Walking Skeleton (pre-existing) | - not found | No walking skeleton exists yet — Slice 00 was a spike (no production code, governed by `.claude/rules/spike.md`), and Slice 01 (this DISTILL's WS target) has not been built. This DISTILL authors the WS scenario for the first time |
| 6 | DEVOPS wave-decisions | - not found | `docs/feature/microvm-driver-cloud-hypervisor/devops/` does not exist |
| 6b | Deliverable Type | + read | `.nwave/des-config.json` — absent; global config absent; Rust workspace with `Cargo.toml` at root → resolves to `application` per ADR-PST-002 precedence and the project's own stated deliverable shape |
| 7 | DESIGN (feature-delta.md, three dispatches) | + read, in full | Titan (system/infrastructure, lines 2653–3364), Hera (domain/bounded-context, lines 3365–3751), Morgan (application/component, lines 3752–4215); Changelog (4216–4247, summarized); two adversarial-review sections (4247–end) confirmed already-incorporated into the design text (not separately re-read line-by-line — every correction they raised is marked "revised"/"corrected" inline in the sections above) |
| 8 | ADR-0081 / ADR-0082 / ADR-0083 | + read | Three Ending Classes; `Vmm` port + `VmConfig`; `DriverRegistry` + `VmReclamation`. Exact `rust` code blocks extracted verbatim for scaffold + scenario fidelity |
| 9 | `docs/architecture/atdd-infrastructure-policy.md` | + read (inherit mode) | Existing file, already adapted to this Rust workspace's Tier 1–4 model with a polyglot-note override of the generic Python DISTILL machinery. Applied as-is; four new port rows appended (see § Project Infrastructure Policy below) |
| 10 | Existing test-placement precedent | + read | `crates/overdrive-cli/tests/integration/exec_spec_walking_skeleton.rs` (the closest existing walking-skeleton shape), `crates/overdrive-cli/CLAUDE.md` (the no-subprocess firm rule — corrected an initial drafting error in `test-scenarios.md`, see § Corrections below), `crates/overdrive-core/traits/driver.rs` (confirms `[G2]`: `DriverType::Vm` already exists, only `MicroVm` awaits deletion), `crates/overdrive-{cli,core}/tests/{acceptance,integration}.rs` entrypoints |
| 11 | Prior DISTILL precedent in THIS repo | + read | `docs/feature/service-vip-allocator/distill/` (brownfield: scaffolding deferred to DELIVER, DWD-05), `docs/feature/phase-1-first-workload/distill/wave-decisions.md` (greenfield: two new crates scaffolded with `panic!`-bodied stubs, DWD-6) — both consulted to calibrate this DISTILL's own scaffolding scope, see DWD-06 below |

---

## Wave-Decision Reconciliation

**Reconciliation passed — 0 contradictions**, per the orchestrator's own pre-verified summary and independently confirmed by a full read of the feature-delta.md DESIGN sections. The three DESIGN dispatches (Titan → Hera → Morgan) are themselves a chain of deliberate, explicitly-marked revisions (the Bar-2 ruling propagated forward through all three, each correction marked "revised"/"withdrawn"/"corrected" in place — never silently edited over). No open contradiction remains anywhere in the chain.

| DISCUSS decision | DESIGN resolution | Contradiction? |
|---|---|---|
| `[D1]`–`[D8]` (all eight DISCUSS decisions) | Consumed verbatim by all three DESIGN dispatches; zero reversals | No |
| SD-1 initial recommendation: converge-on-boot (Bar 1) reap | User ruling 2026-08-11: Bar 2 registered `Reconciler` | Not a contradiction — a **superseding user ruling**, propagated consistently through Hera's DD-1(b) and Morgan's `VmReclamation` design; every downstream artifact reflects the ruling |
| Guardrail 2 / Guardrail 3 (day-count lift triggers) | Retired by user ruling 2026-08-11, citing CLAUDE.md § "No effort/time budget cuts" | Not a contradiction — a **standing project-policy correction**, not a design reversal |
| GH #264 "BYO rootfs deferred to DEVOPS" (changelog row) | Closed as wrong-premised the same day — BYO rootfs is a slicing mechanism (intake I-3), not a product surface | Already-reconciled per the orchestrator's dispatch; not re-raised |
| C-1…C-7 (spike-vs-slice-text contradictions) | All seven are **slice-text and AC corrections**, explicitly not re-slicing events | Resolved by design; carried into `test-scenarios.md` as `@correction:C-N` tags |

**Per the orchestrator's explicit instruction, the following are NOT re-raised as open items** (already reconciled, and re-litigating them would itself be a DISTILL error): the `superseded-by-DESIGN` markers on the slice briefs, the D-5 `shmem_enabled` undeferral, the day-count guardrail supersession, and GH #264's closure.

---

## Graceful Degradation

| Artifact | Status | Action |
|---|---|---|
| DEVOPS | Missing | Default environment matrix applied per the orchestrator's dispatch: `clean` / `with-pre-commit` / `with-stale-config` is NOT directly applicable to this feature's real concern (a real Cloud Hypervisor + kernel + rootfs artifact substrate), so the effective default environment matrix is Slice 00's own two-kernel matrix (Lima dev kernel + pinned 6.18 appliance kernel), already established by the spike and carried forward as the Tier-3 execution envelope for every scenario in `test-scenarios.md` |
| Journeys | Missing (`docs/product/journeys/`) | DISCUSS's own Persona + JTBD sections substitute; no scenario traceability lost — every scenario traces to a user story (US-VM-1…9) instead |
| KPI Contracts | Missing (`kpi-contracts.yaml` does not exist) | KPI targets taken from feature-delta.md § Outcome KPIs (K1–K10, fully specified with baseline/measurement plan). See § KPI Traceability in `test-scenarios.md` |

None of these trigger a BLOCK — per the Graceful Degradation Matrix, only a missing DESIGN artifact blocks, and DESIGN is present (and unusually thorough: three dispatches, two adversarial-review iterations, two ADRs, ~1900 lines of `brief.md` additions).

---

## Decisions

### DWD-01: Walking Skeleton — ONE scenario, Slice 01, US-VM-1's happy path

Per the orchestrator's explicit instruction and per Mandate 5: exactly one `@walking_skeleton`-equivalent scenario (S-VM-01), driven end-to-end through the real production entry points (`overdrive deploy` → real in-process `overdrive serve` → real `cloud-hypervisor` subprocess spawned by `VmDriver` → real guest kernel → real `overdrive-init` beacon over vsock). No test in this scenario installs, binds, programs, or supplies anything `run_server` does not supply itself — `DriverRegistry` composition (discover → probe → insert) happens inside `overdrive serve`'s own boot sequence.

The remaining four US-VM-1 UAT scenarios (S-VM-02…05) are **not** additional walking skeletons — they are focused Slice-01 scenarios sharing the same driving port and fixture shape, per Mandate 5's "2-3 skeletons + 15-20 focused scenarios" ratio (this feature needed exactly 1 skeleton; its walking-skeleton-adjacent focused scenarios cover the remaining happy/error/edge shapes at the same driving port).

### DWD-02: Adapter Strategy — Architecture of Reference, specialized per this project's Project Infrastructure Policy

The generic skill's Strategy A/B/C/D framing does not apply (retired per the skill itself, replaced by the Architecture-of-Reference + Project-Infrastructure-Policy model — but even THAT model is Python/pytest-bdd-flavored and maps onto this Rust project's actual four-tier discipline as follows:

| Port class (generic skill) | This project's mapping | Mechanism |
|---|---|---|
| Driving (entry point) | `overdrive deploy` / `overdrive workload describe` / `overdrive job stop` / `overdrive serve` boot | Direct CLI handler call (`overdrive_cli::commands::*`) against a REAL in-process `overdrive serve` — **never a subprocess**, per `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess" (a firm rule this DISTILL's first draft violated and had to correct — see § Corrections below) |
| Driven internal (real) | `Vmm`, `VmHostState`, `CgroupAccounting` (production adapters), the spec parser, `JobEnvelope` rkyv | `cargo xtask lima run --` as root, `--features integration-tests`, against a real `cloud-hypervisor` binary + real cgroupfs + real vsock. Tier 3 per `.claude/rules/testing.md` |
| Driven external / non-deterministic | None new. `Vmm`/`VmHostState` substrate LIES (Landlock flag/LSM absence, KVM unreachability, an unbindable run root, an unavailable sandbox) are injected via `SimVmm`/`SimVmHostState` fault scripting **at the port boundary** — sanctioned by system constraint 1 ("a `Sim*` adapter injected at a port boundary is fine") because the whole Lima test envelope runs one kernel and these specific capability facts cannot be produced without swapping it. **Corrected 2026-08-11 (DWD-11): the non-reflink fault class is EXCLUDED from this row — it is NOT injected.** The original text here claimed "no genuinely lying host exists in [the Lima] envelope" for the substrate-LIE surface as a whole, including non-reflink; that claim was false for non-reflink specifically. A tmpfs directory or a loopback-mounted ext4 image is a REAL, non-injected non-reflink filesystem, one mount command away, inside the same root Lima harness this suite already uses (`overdrive-testing`) — and is exercised for real at S-VM-75 (boot probe) and S-VM-94 (per-launch clone). See `test-scenarios.md` S-VM-13's crafter note for the corrected scope split | `SimVmm` / `SimVmHostState` (`overdrive-sim`), fault-injection scripting mirroring the existing `SimMtlsIntercept` pattern already recorded in `atdd-infrastructure-policy.md`, for the capability-flag / run-root / sandbox classes ONLY |

### DWD-03: Project Infrastructure Policy — inherit mode, six new port rows appended

`docs/architecture/atdd-infrastructure-policy.md` **exists** (already adapted to this Rust workspace, carrying the same polyglot-note override this feature needs). Applied as `--policy=inherit` (default). **Six** new rows appended under "Driving" (1 row) / "Driven internal (real)" (3 rows) / "Driven external / non-deterministic (fake)" (2 rows) — corrected 2026-08-11 during the adversarial-review remediation pass (DWD-11): the original text said "four new rows," which was a plain miscount against the markdown block immediately below it (1 + 3 + 2 = 6, not 4):

```markdown
## Driving
| `overdrive deploy <spec.toml>` — `[vm]` driver table | direct CLI handler call (`overdrive_cli::commands::deploy::deploy`), real in-process `overdrive serve`, real Cloud Hypervisor VMM spawned as a child process, run under `cargo xtask lima run --` as root | microvm-driver-cloud-hypervisor #42. Same driving port as every prior `[exec]` feature — `[vm]` adds no new CLI surface |

## Driven internal (real)
| `Vmm` (`CloudHypervisorVmm`) | real `cloud-hypervisor` subprocess + real vsock UDS + real Landlock/seccomp/cgroup, inside Lima as root | #42. `vmm_equivalence.rs` is the cross-adapter contract enforcement |
| `VmHostState` (`RealVmHostState`) | real cgroupfs read + real filesystem enumeration of the VM run root + clone directory | #42. `vm_host_state_equivalence.rs` |
| `CgroupAccounting` (real `memory.events` read) | real cgroupfs, post-mortem read only (never a live subscription — D-3 fold-in's deliberately reduced scope) | #42 |

## Driven external / non-deterministic (fake)
| `Vmm` — the substrate LIE surface (non-reflink staging dir, unavailable `--landlock`, missing `cloud-hypervisor` capability flags) | `SimVmm` fault scripting, mirroring the existing `SimMtlsIntercept` STANDING-fault pattern | #42. The whole Lima test envelope runs one kernel; no genuinely lying host exists in it, so the fault is injected at the port boundary per system constraint 1 |
| `VmHostState` — the sandbox/settle-contract LIE surface (`--sandbox=namespace` unavailable) | `SimVmHostState` fault scripting | #42 |
```

### DWD-04: Test Crate Placement

| Scenario scope | Crate | Path | Rationale |
|---|---|---|---|
| Walking skeleton + all Tier-3 CLI-driven scenarios (S-VM-01…05, 09, 11…15, 19, 33…66, 68) | `overdrive-cli` | `tests/integration/vm_*.rs`, gated `integration-tests` | Real driving port is the CLI handler; mirrors `exec_spec_walking_skeleton.rs`. **Corrected 2026-08-11 (DWD-13): excludes S-VM-67**, which the original range silently swept in — S-VM-67 is no longer Tier-3/CLI-driven, see the DWD-13 addendum below. **Corrected 2026-08-11 (DWD-16): explicitly names S-VM-09 and S-VM-19** — both sit in sub-range gaps (06-10, 16-32) the span notation silently omitted; both carry the identical `overdrive deploy` / real in-process `overdrive serve` driving port and `@tier3 @real-io` tags as their range neighbours, so this is an explicit-naming fix, not a re-placement — see DWD-16. **Added 2026-08-11 (DWD-17): this row's file set is a MIXED capability lane** — most of its members carry `@requires-kvm` (a real `cloud-hypervisor` guest-boot attempt), but S-VM-11, 12, 13, 33, 34, 35, 38, 40, 41, 51, 58, 59 do NOT (composition-gate probes, `SimVmm`-injected faults, and pre-spawn artifact-validation rejections — none of these spawn a real guest-booting hypervisor). `integration-tests` gates the whole file at the Lima/real-I/O level; `@requires-kvm` is the narrower, flakier sub-capability the concurrent roadmap pass' preflight mechanism must gate independently, per-test-function, within this same file set — see DWD-17 |
| Pure-function `@property` scenarios (`VmConfig` value family, `plan_reclamation`, `SupervisionSet`, vCPU derivation) | `overdrive-core` | `tests/acceptance/vm_*.rs`, default lane | Port-to-port at function scope; no I/O, no Lima needed |
| Parse-boundary rejections (S-VM-06/07/62) | `overdrive-core` | `tests/acceptance/vm_spec_driver_table_dispatch.rs`, default lane | In-process TOML deserializer, no subprocess and no real VMM needed |
| `JobEnvelope` V1→V2 schema evolution (S-VM-10) | `overdrive-core` | `tests/schema_evolution/workload_intent.rs` (EXISTING file, edited, not a new file) | Per the six-step version-bump procedure; `FIXTURE_V1` never touched |
| `Vmm` / `VmHostState` adapter equivalence (S-VM-90/91) | `overdrive-host` | `tests/integration/vmm_equivalence.rs`, `vm_host_state_equivalence.rs`, gated `integration-tests` | Named exactly by the design (ADR-0082 §D6, ADR-0083 §D7). **Added 2026-08-11 (DWD-17): split capability lane** — `vmm_equivalence.rs` (S-VM-90) drives real `CloudHypervisorVmm::create()`, `@requires-kvm`; `vm_host_state_equivalence.rs` (S-VM-91) operates on generic cgroupfs/filesystem host-state primitives, no guest-boot needed — see DWD-17 |
| `VmReclamation` reconciler in-memory shapes (S-VM-21 companion, 24, 26, 27, 29) | `overdrive-core` | `tests/acceptance/vm_reclamation_*.rs`, default lane | Pure `reconcile()` driving port |
| `VmReclamation` reconciler Tier-3 shapes (S-VM-21, 22, 23, 25, 28, 30) | `overdrive-cli` | `tests/integration/vm_reclamation_tier3.rs`, gated `integration-tests` | Real `overdrive serve` convergence loop. **Added 2026-08-11 (DWD-17): every member of this row carries `@requires-kvm`** — the reconciler's Tier-3 companion shapes are specifically meant to prove convergence against genuinely real leftover VM artifacts (per this project's "No Fixture Theater" rule), not fixture-crafted stand-ins — see DWD-17 |
| DST invariants (`VmReclamationConverges`, `SupervisedVmSurvivesEveryTick`, `VmReclamationIdempotentSteadyState`, `EndingInFlightIsNeverReclaimed`) | `overdrive-sim` | `src/invariants/vm_reclamation.rs` | Per the existing `Invariant` catalogue mechanical shape (`ALL`, `as_canonical`, `harness.rs` dispatch). Covers S-VM-87/88/89 (S-VM-24 already placed above) |
| `overdrive-init` guest agent | `overdrive-init` (NEW crate, `binary` class) | `src/main.rs` | `[D4]`; a new workspace member |

**Added 2026-08-11 (adversarial-review remediation, DWD-11) — crate/file placement for the thirteen new scenarios and the two contested-scenario rewrites:**

| Scenario scope | Crate | Path | Rationale |
|---|---|---|---|
| `MtlsInterceptWorker` gating, ungated-off arm (S-VM-74) | `overdrive-cli` | `tests/integration/vm_walking_skeleton.rs`, gated `integration-tests` | Same driving port and fixture family as S-VM-05, which it extends. `@requires-kvm` (DWD-17) — depends on the allocation reaching Running |
| `Vmm` real non-reflink substrate (S-VM-75) | `overdrive-cli` | `tests/integration/vm_walking_skeleton.rs`, gated `integration-tests` | Boot-time scenario, alongside S-VM-11…13. NOT `@requires-kvm` (DWD-17) — the FICLONE probe refuses the boot before any guest-booting hypervisor capability exists |
| `VmDriver::stop` totality against `SimVmm` (S-VM-76) | `overdrive-worker` | `tests/acceptance/vm_driver_stop_totality.rs` (NEW file), default lane | Component-scope, no Lima/real-CH needed — `SimVmm` only |
| Hydration read order / P2-over-`VmReclamation` (S-VM-78, 80) | `overdrive-core` | `tests/acceptance/vm_reclamation_plan_purity.rs` (EXISTING file, extended) or a new sibling `vm_reclamation_claim_lifecycle.rs` — DELIVER's own call once the `VmReclamationState`/`VmSupervision` types exist | In-memory, pure/component-scope, same family as S-VM-31/32/92. **Corrected 2026-08-11 (DWD-16): S-VM-77 and S-VM-79 removed from this row** — see the new `overdrive-control-plane` row directly below |
| Abandonment boundary — claim release across `RetryOutcome` arms — / write-time terminality guard over `execute_reclaim_allocation` (S-VM-77, 79) | `overdrive-control-plane` | `tests/acceptance/vm_reclamation_claim_lifecycle.rs` (NEW file, default lane) — exact filename DELIVER's own call once `VmSupervision`/`execute_reclaim_allocation` exist, following the same file-TBD convention this table already uses elsewhere (e.g. the S-VM-67 row in `test-scenarios.md`'s Driving Ports table) | **Added 2026-08-11 (DWD-16).** Both driving ports are `overdrive-control-plane`-resident production code — `worker/exit_observer.rs`'s loop body (confirmed at `crates/overdrive-control-plane/src/worker/exit_observer.rs`) and the `action_shim` executor for `ReclaimAllocation` (`crates/overdrive-control-plane/src/action_shim/`, this crate's own existing per-action executor-module convention — `action_shim/issue_svid.rs`, `action_shim/release_service_vip.rs`, etc.). `overdrive-control-plane` depends on `overdrive-core`, never the reverse (verified: `overdrive-core`'s `Cargo.toml` carries no `overdrive-control-plane`/`overdrive-cli` dependency edge) — an `overdrive-core`-placed test cannot import either surface. Default lane (`tests/acceptance/*.rs`, unwired from `integration-tests`), matching S-VM-77/79's existing `@tier1 @in-memory` tags — no tier change required |
| Fourth evaluation — `svid_lifecycle` (S-VM-81) | `overdrive-cli` | `tests/integration/vm_reclamation_tier3.rs`, gated `integration-tests` | Real `overdrive serve` convergence loop, mTLS-composed. `@requires-kvm` (DWD-17) — the SVID-holding allocation must have been a genuinely running VM |
| ESR invariant scenarios (S-VM-87, 88, 89) | `overdrive-sim` | `src/invariants/vm_reclamation.rs` | Same file as the pre-existing four-invariant placement above |
| `CgroupAccounting` adapter equivalence (S-VM-93) | `overdrive-host` | `tests/integration/cgroup_accounting_equivalence.rs`, gated `integration-tests` | Named exactly by ADR-0082 §D8, same shape as `vmm_equivalence.rs` / `vm_host_state_equivalence.rs`. NOT `@requires-kvm` (DWD-17) — generic cgroupfs read-once semantics, no guest-boot needed |
| Per-launch `FICLONE` self-application (S-VM-94) | `overdrive-host` | `tests/integration/vmm_ficlone_per_launch.rs` (NEW file), gated `integration-tests` | Real non-reflink fixture, adapter-level, not through the CLI. NOT `@requires-kvm` (DWD-17) — the scenario's own `Then` states no hypervisor process was spawned |
| S-VM-35 (rewritten to the TOCTOU window) | `overdrive-cli` | `tests/integration/vm_walking_skeleton.rs`, gated `integration-tests` | Unchanged placement — the rewrite changes the scenario's content, not its file. NOT `@requires-kvm` (DWD-17) — the binary is gone before spawn is attempted, exec fails immediately |
| S-VM-13 (narrowed to capability-flag classes) | `overdrive-cli` | `tests/integration/vm_walking_skeleton.rs`, gated `integration-tests` | Unchanged placement. NOT `@requires-kvm` (DWD-17) — `SimVmm`-injected, no real `cloud-hypervisor` process is involved |

### DWD-05: Scenario Coverage Shape

**Recomputed 2026-08-11 (adversarial-review remediation, DWD-11)** after 13 new scenarios landed (S-VM-74…81, 87…89, 93, 94) and one `@error_path`/`@edge_case` double-tag on S-VM-76 was resolved to a single tag:

| Category | Count | % |
|---|---|---|
| `@happy_path` | 18 | 21% |
| `@error_path` | 40 | 46% |
| `@edge_case` | 12 | 14% |
| No happy/error/edge tag (pure `@property`/`@example` scenarios) | 17 | 20% |
| `@property` | 20 | 23% |
| `@example` (fixed call sequences at layer 3, per Mandate 9) | 4 | 5% |
| **Total distinct scenarios** | **87** | — |

Counts recomputed mechanically (`grep -c '^\*\*Tags\*\*:'` cross-checked
against `grep -c '^#### S-VM-'`, both 87; per-tag counts via
`grep '^\*\*Tags\*\*:' | grep -c '@<tag>'`) — this is the second such
mechanical recount this feature has needed: the original DISTILL pass
caught a 2-scenario undercount (72→74) via its own `@contract-shape:`
tagging pass (DWD-08); this remediation pass's addition of 13 scenarios
required the same re-verification discipline, and additionally caught S-VM-76
carrying both `@error_path` and `@edge_case` (resolved to `@error_path`
alone, matching the S-VM-47 precedent for the same "unresponsive guest"
shape) before the counts would sum correctly (70+17=87, not 71+17=88).

Error + edge coverage: **52 of 87 ≈ 60%** — well above the 40% target, and structurally so: this feature's north-star KPI (K1) and four of its five guardrail KPIs (K2, K7, K9, plus the DST safety invariant `SupervisedVmSurvivesEveryTick`) are ALL about refusing to ship a lie, which is inherently an error/edge concern — and the remediation pass's additions (the NEW-1 pins, the fourth evaluation, both missing adapter-equivalence tests) skew the same direction.

### DWD-06: Mandate 7 Scaffolding — SCOPED to Slice 01, safe-shape placeholders only

**Decision: scaffold, but narrowly, and calibrated against two conflicting precedents already in this repo.**

`docs/feature/service-vip-allocator/distill/wave-decisions.md` DWD-05 defers ALL Rust scaffolding to DELIVER, reasoning "brownfield refactor of an existing isolated primitive." `docs/feature/phase-1-first-workload/distill/wave-decisions.md` DWD-6 scaffolds two full NEW crates with `panic!`-bodied stubs up front, reasoning "DESIGN named the crate boundaries; DISTILL lands the empty crates so the test paths in `test-scenarios.md` have something to compile against."

This feature is **greenfield** (a new crate `overdrive-init`, three new port traits `Vmm`/`VmHostState`/`CgroupAccounting`, a new reconciler `VmReclamation`, thirteen new `TransitionReason` variants, a new beacon Published Language) — closer in shape to `phase-1-first-workload` than to `service-vip-allocator`. But its signature surface is an order of magnitude larger and more precisely pinned (two ADRs + ~1900 lines of `brief.md`), which changes the calculus:

1. **The RED-scaffold TEST bodies are 100% safe to author regardless of surface size** — per `.claude/rules/testing.md` § "RED scaffolds", a scaffold's body is a bare `panic!("Not yet implemented -- RED scaffold (...)")` behind `#[should_panic(expected = "RED scaffold")]`. It requires ZERO knowledge of the not-yet-existing production types, so there is no risk of inventing API surface by writing one. **These ARE authored, for a subset of Slice 01** (S-VM-01…08, S-VM-16…18, S-VM-20 — ten of the twenty Slice 01 scenario IDs 01-20, plus S-VM-06/S-VM-07 from AC-02) plus the three cross-cutting pure-function property scenarios (S-VM-31/32/92) — **fifteen scaffolds across four files**, all verified to compile clean (`cargo check`, `cargo clippy -D warnings`) and to classify RED, not BROKEN (`cargo nextest run` — 15/15 pass, panic message matched). **Correction, 2026-08-11 (adversarial-review remediation, DWD-11):** the original text here claimed the fifteen scaffolds covered "every scenario in Slice 01 (S-VM-01…20) plus the three cross-cutting" — that phrase was self-contradictory on its own numbers (20 + 3 = 23, not 15) and also factually wrong: only **twelve** of the twenty Slice-01 scenario IDs (01, 02, 03, 04, 05, 06, 07, 08, 16, 17, 18, 20) are scaffolded; S-VM-09, 10, 11, 12, 13, 14, 15, 19 are NOT. The twelve Slice-01 IDs plus the three cross-cutting IDs correctly sum to fifteen, matching the actual file contents (`vm_walking_skeleton.rs` five tests, `vm_config_pure_functions.rs` five tests, `vm_spec_driver_table_dispatch.rs` two tests, `vm_reclamation_plan_purity.rs` three tests — verified by `grep -n '^fn \|^async fn '` against all four files during the remediation pass). No scaffold files were changed by this correction — only the prose describing them.
2. **Production-side `todo!()` stub MODULES (the `Vmm` trait, `VmConfig`'s ~8 value types, `VmHostState`, `VmReclamation`'s full reconciler wiring into `AnyReconciler`/`AnyState`/`AnyReconcilerView`) are NOT authored by this DISTILL wave.** Two reasons, stated so neither is mistaken for laziness:
   - Every one of those signatures is **already exactly pinned**, verbatim, in ADR-0082, ADR-0083, and `brief.md` §§ 101–105a — DELIVER's RED phase reads those documents directly (per CLAUDE.md § "Implement to the design"). A DISTILL-authored parallel transcription of the same signatures is a **second, driftable source of truth** for a fact that already has exactly one — precisely the anti-pattern this project's own design repeatedly refuses elsewhere (`DriverRegistry`'s missing-map-entry-not-a-bool rationale; `SupervisionSet::Unavailable`-as-`Default` rationale). If this DISTILL's transcription and the ADR ever diverged, a crafter would not know which one to trust.
   - Wiring `VmReclamation` into the five compiler-enforced enum/match sites `brief.md` §105a.9 names (`AnyReconciler`, `AnyState`, `AnyReconcilerView`, `AnyReconciler::reconcile`'s 4-tuple match, `hydrate_desired`/`hydrate_actual`'s matches, `dispatch_single`'s match) is a genuine, non-trivial, ADR-pinned edit to five EXISTING production files this session did not audit line-by-line. Per this repo's own `.claude/rules/testing.md` § "Downstream fallout on pre-existing tests is expected and correct," that fallout is the CORRECT shape for DELIVER's RED phase to produce and resolve — not something DISTILL should pre-empt from outside a verified read of every touched site.
3. **What IS authored** (verified compiling + RED, not narrated):
   - `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs` — S-VM-01…05 (US-VM-1's five UAT scenarios), wired into `tests/integration.rs`.
   - `crates/overdrive-core/tests/acceptance/vm_config_pure_functions.rs` — S-VM-08, 16, 17, 18, 20 (the `VmConfig` anti-corruption mandatory-mutation-target pure functions).
   - `crates/overdrive-core/tests/acceptance/vm_spec_driver_table_dispatch.rs` — S-VM-06, 07 (exactly-one-driver-table parse rejection).
   - `crates/overdrive-core/tests/acceptance/vm_reclamation_plan_purity.rs` — S-VM-31, 32, 92 (`plan_reclamation` purity, Ending Class totality, `SupervisionSet::reclamation_authorised`).
   - All four wired into their crate's `tests/{acceptance,integration}.rs` entrypoint per this project's `mod {acceptance,integration} { mod x; }` convention.
4. **What is deferred to DELIVER, and exactly where**: the remaining Slice 01 AC-derived scenarios not yet scaffolded (**S-VM-09, 11, 12, 13, 14, 15, 19** — corrected 2026-08-11, DWD-11: the original list here omitted S-VM-11 and S-VM-12, the SD-5 composition-gate scenarios, entirely — they appeared on neither this deferred list nor the scaffolded-set list in point 1, an accounting gap the remediation pass's mechanical recount surfaced), the `JobEnvelope` V1→V2 fixture (S-VM-10 — deliberately NOT touched here, see below), the AC-18/AC-19/AC-20 scenarios added by the remediation pass (S-VM-74…81, 87…89, 93, 94 — none of these existed at the time of the original DISTILL pass and none are scaffolded by it either, for the identical reasoning given in this point), Slices 02–05 in full, the cross-cutting `VmReclamation` reconciler's remaining scenarios (S-VM-21…30 in-memory + Tier-3 shapes), the four DST `Invariant` variants, and the `vmm_equivalence.rs` / `vm_host_state_equivalence.rs` / `cgroup_accounting_equivalence.rs` contract tests. Every one of these is fully specified in `test-scenarios.md` with its exact crate/file placement already decided in DWD-04 (extended below for the remediation pass's additions) — DELIVER's RED phase scaffolds each at the start of its own slice, one scenario at a time, per this project's "one test at a time" TDD discipline. This is a **bounded, per-slice** version of the `service-vip-allocator` deferral, not the same blanket deferral — the difference is that DWD-04 above already commits to exact file paths for every one of them, closing the "no compile target" gap DWD-6's `phase-1-first-workload` reasoning was written to prevent.
5. **`JobEnvelope` V1→V2 is deliberately NOT scaffolded in this DISTILL wave.** The six-step version-bump procedure (`.claude/rules/development.md` § "rkyv schema evolution") operates on an EXISTING file (`crates/overdrive-core/tests/schema_evolution/workload_intent.rs`) whose `FIXTURE_V1` constant must never be touched. This session did not `Read` that file's current contents (out of scope for the scaffold pass), and per this project's own `Read`-before-`Edit` discipline, a same-session edit without that read would be premature. DELIVER's Slice 01 RED phase reads the file first, then executes the six-step procedure in a single commit.

**Evidence the scaffolded set is genuinely RED, not BROKEN** (per Mandate 7's "Verify RED Classification" step):

```
cargo xtask lima run -- cargo check -p overdrive-core --tests                         → clean
cargo xtask lima run -- cargo check -p overdrive-cli --tests --features integration-tests → clean
cargo xtask lima run -- cargo clippy -p overdrive-core -p overdrive-cli --tests \
  --features integration-tests -- -D warnings                                        → clean
cargo xtask lima run -- cargo nextest run -p overdrive-core \
  -E 'test(vm_config_pure_functions) or test(vm_reclamation_plan_purity) or test(vm_spec_driver_table_dispatch)'
  → 10 tests run: 10 passed
cargo xtask lima run -- cargo nextest run -p overdrive-cli --features integration-tests \
  -E 'test(vm_walking_skeleton)'
  → 5 tests run: 5 passed
```

Fifteen scaffolds, fifteen passes (the `#[should_panic(expected = "RED scaffold")]` fires as designed on every one) — genuine RED, verified by execution, not narrated.

### DWD-06a: USER RULING (2026-08-11, adversarial-review remediation) — the 59-of-74-scenario (now 72-of-87) scaffold deferral STANDS; project rule governs over the generic skill's ADR-025 statement

`nw-acceptance-designer-reviewer` (Sentinel) raised a BLOCKER against DWD-06
in its adversarial review: the `nw-distill` skill's own text states, under
"ADR-025 (2026-05-07) — DISTILL is canonical AT author," that *"DISTILL
produces ALL acceptance tests as scaffolded RED"* — and DWD-06 above
scaffolds only 15 of 74 (now 15 of 87, unchanged in count by this
remediation pass — the 13 new scenarios join the deferred set, none of them
scaffolded here either, per the same reasoning DWD-06 already gives).
Read literally, DWD-06's deferral contradicts that statement.

**The user has ruled: the deferral is correct and stands, and this is not
re-litigated by any later dispatch.** The reasoning:

`.claude/rules/testing.md` is a **checked-in project rule**, not a
suggestion, and it says explicitly, in its own voice: *"The crafter
translates those scenarios into Rust integration tests in
`crates/{crate}/tests/acceptance/*.rs` (or `tests/*.rs`)."* That sentence
assigns AT-to-Rust translation to **the crafter** (DELIVER), not to DISTILL.
Per CLAUDE.md (the project's own root instructions, which this session
operates under): *"project instructions override defaults."* The generic
`nw-distill` skill's ADR-025 line is a cross-project **default**; a
checked-in `.claude/rules/*.md` file in THIS repo is a **project
instruction**. Where the two disagree — as they do here — the project
instruction governs. This is not a judgment call DISTILL is making
unilaterally; it is CLAUDE.md's own precedence rule applied to a case where
it actually bites.

Two things follow, and both were left open by the earlier text:

1. **The deferral is the correct DEFAULT posture for this project, not a
   scoping shortcut taken because the surface was large.** DWD-06's own
   stated reasons (a same-fact-two-sources risk against the ADR/brief's
   already-pinned signatures; genuine multi-site production edits this
   session did not audit) are real and independently sufficient, but they
   are not what settles the question — `testing.md`'s assignment of the
   translation step to the crafter is what settles it, and it would settle
   it even for a feature with a much smaller signature surface.
2. **Ownership, answered explicitly** (Sentinel's finding correctly noted
   this was left open): **the crafter (`nw-software-crafter`) authors each
   slice's Rust scaffolds**, one slice at a time, at the start of that
   slice's own RED phase — reading the exact GIVEN/WHEN/THEN and driving
   port from `test-scenarios.md` and the exact pinned signature from the
   cited ADR/`brief.md` section (never inventing one, per CLAUDE.md §
   "Implement to the design"). **The reviewer of each such scaffold is
   `nw-software-crafter-reviewer`**, at DELIVER's own Phase 4 review gate
   (per `nw-deliver`'s per-step review cadence) — NOT
   `nw-acceptance-designer-reviewer` (Sentinel), whose review scope is
   DISTILL's own output (this file, `test-scenarios.md`, and the 15
   scaffolds DISTILL itself authored). A DELIVER-authored scaffold that
   diverges from `test-scenarios.md`'s GIVEN/WHEN/THEN is therefore a
   DELIVER-wave defect, caught by DELIVER's own review gate — not a gap in
   this DISTILL artifact.

This ruling is recorded here so it is not re-raised by a future reviewer
reading only the skill's generic ADR-025 line without also reading this
project's `testing.md`.

### DWD-07: Corrections made during this DISTILL pass

Two drafting errors were caught and corrected before this document was finalized, both worth recording so the reasoning is not silently lost:

1. **The "real subprocess" framing was wrong for every CLI-driven scenario.** The first draft of `test-scenarios.md` described every Tier-3 scenario's driving port as "`overdrive deploy` (real subprocess)". `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess" is a **firm rule**: `overdrive-cli` tests call `overdrive_cli::commands::*` handler functions directly against a real in-process `overdrive serve`, never `Command::new(CARGO_BIN_EXE_overdrive)`. Corrected globally (48 occurrences) before this DISTILL was finalized. The REAL OS-level subprocess in every one of these scenarios is `cloud-hypervisor` itself — spawned by `VmDriver` inside the real, in-process `overdrive serve` — which is what actually makes them Tier-3 / `@real-io`; the CLI layer's own invocation shape is unrelated to that classification.
2. **`SupervisionSet::reclamation_authorised` and the two adapter-equivalence tests (`vmm_equivalence.rs`, `vm_host_state_equivalence.rs`) were referenced in the Driving Ports table before being defined as scenarios.** Added as S-VM-90/91/92 under a new "Cross-cutting — Port contract enforcement" section before finalizing, closing the dangling reference.

### DWD-09: dst-lint AC ownership — five clauses, each is an `xtask` unit test, none is a Rust acceptance-test scaffold

`brief.md` §113 states, of five structural clauses (`--disk` never rendered
outside `DiskAttachment::to_disk_arg`; Landlock rules never built outside
`VmRunDir::landlock_grant`; `--seccomp` never rendered outside
`VmConfinement::seccomp_arg`; `MemoryPlan` never struct-literal-constructed;
the exit observer's loop body contains exactly one `release_supervision`
call outside `match outcome`): *"the three lint clauses are Slice 01
deliverables with an acceptance criterion, not recommendations."*
`nw-acceptance-designer-reviewer` correctly flagged that no AC or scenario
in `test-scenarios.md` names them.

**Decision: each clause is owned by a `#[test]` in `xtask/src/dst_lint.rs`'s
`mod tests`, following the EXISTING `CrashObservabilityStructLiteral`
clause's shape verbatim** (`xtask/src/dst_lint.rs:2283`, `:2722`, `:4428`) —
NOT a new `.feature`-shaped Rust acceptance-test scaffold. This is a
mechanical consequence of this project's own test taxonomy
(`.claude/rules/testing.md`), not a DISTILL judgment call: `xtask` is dev
tooling — per project memory, "xtask excluded from mutation testing" — and
every existing AST-lint clause in this codebase is verified by a unit test
that feeds the scanner synthetic fixture source and asserts the emitted
`BannedKind` violation, not by an acceptance test against a driving port.
Minting a `.feature`-shaped scenario for a static-analysis gate would
misclassify a compiler-adjacent tool as a runtime behaviour.

Each clause's DELIVER-step ownership, and the behavioral scenario it sits
beside (added as crafter-note pointers in `test-scenarios.md` during this
remediation pass, DWD-11):

| Clause | DELIVER step (introduces the type the clause guards) | Behavioral scenario it complements |
|---|---|---|
| `"--disk"` never rendered outside `DiskAttachment::to_disk_arg` | The step landing `DiskAttachment` (Slice 01, ADR-0082 §D2.1) | S-VM-16 |
| Landlock rules never built outside `VmRunDir::landlock_grant` | The step landing `VmRunDir` (Slice 01, ADR-0082 §D2.2) | S-VM-53 |
| `"--seccomp"` never rendered outside `VmConfinement::seccomp_arg` | The step landing `VmConfinement` (Slice 01, ADR-0082 §D2) | S-VM-08 |
| `MemoryPlan` never struct-literal-constructed | The step landing `MemoryPlan::derive` (Slice 01, ADR-0082 §D2.3) | S-VM-18 |
| Exit observer releases on every arm, outside `match outcome` | The step landing the `VmSupervision` claim lifecycle (cross-cutting, brief §105a.3) | S-VM-77 |

### DWD-10: Kernel matrix ownership — the pinned-6.18-appliance leg is a Tier-3 CI concern, not a per-scenario tag

`.claude/rules/testing.md` § "Tier 3 — Real-Kernel Integration" § "Kernel
matrix" declares the merge-blocking envelope as **the pinned 6.18 appliance
kernel PLUS `bpf-next`** (soft-fail); `spike/wave-decisions.md`'s PROMOTE
verdict is itself measured against **both** the Lima dev kernel and that
pinned appliance kernel. No individual scenario in `test-scenarios.md`
names the second leg — every Tier-3 scenario's fixture description says
"under `cargo xtask lima run --`" without distinguishing which kernel the
CI runner boots.

**Decision: this is NOT tagged per-scenario.** Retagging all ~55 Tier-3
scenarios with a `@kernel-matrix` marker would not change which kernel
actually runs them — that is a CI-runner concern (which kernel image
`little-vm-helper` boots), not a scenario-authoring concern, and per-
scenario tags would drift the moment the matrix itself changes (per
`.claude/rules/testing.md`'s own "dropping a kernel requires an ADR" — the
matrix is infrastructure-owned, not test-owned). **Ownership is assigned
here instead**: every Tier-3 scenario in this catalogue runs, unmodified,
against BOTH matrix legs via the project's existing CI Tier-3 LVH lane
(`.claude/rules/testing.md` § "CI topology" row D, `cargo xtask
integration-test vm`) — this is inherited infrastructure, not a new
obligation this feature introduces, and DEVOPS (not DISTILL) is the wave
that would adjust the matrix membership if this feature ever needed a
kernel-specific fixture (it does not — nothing in `test-scenarios.md`
depends on a kernel feature narrower than the pinned floor already
established by `spike/findings.md`).

---

## Reuse Analysis

DISTILL surfaced no contradiction with either DESIGN dispatch's Reuse Analysis table (Titan's system-scope 13-row table, Hera's 16-row domain table, Morgan's 37-row application table — all read in full). Every disposition carries forward verbatim into `test-scenarios.md`'s scenario set:

- `Vmm` port (CREATE NEW, pre-ratified by intake I-2) — mirrored by S-VM-01/02/11–15/90.
- `VmHostState` port (CREATE NEW, SD-1 pin 1) — mirrored by S-VM-21–30/91.
- `VmReclamation` reconciler + two `Action` variants (CREATE NEW at Bar 2, user-ruled) — mirrored by S-VM-21–32.
- `DriverRegistry` (CREATE NEW, ADR-0022's pre-committed migration) — mirrored by S-VM-04/11–13.
- `ExecDriver`'s `pre_exec` + `setns(CLONE_NEWNET)` (REUSE VERBATIM) — mirrored by S-VM-05 (no separate scenario needed for the reuse itself; it is asserted as a property of the VM path).
- `CgroupManager` create-scope → write-limits → enrol-PID (EXTEND, value only) — mirrored by S-VM-05/19.
- `exit_observer::classify` / `WorkloadLifecycle` restart-backoff (REUSE UNCHANGED, substitution of classification INPUT only) — mirrored by S-VM-02/42–45.
- `TransitionReason` / `StoppedBy` (EXTEND, append-only, `#[non_exhaustive]`) — mirrored by S-VM-33–37/58–60/64–67, and the Ending Class disposition (S-VM-32).
- `is_natural_exit` (the ONLY predicate whose MEANING changes) — mirrored by S-VM-26.
- Twelve reuse rows across the two 13-/16-row tables (`is_intentionally_stopped`, `is_restartable`, `WorkloadLifecycleView.restart_counts`, `AllocState`, `ExitEvent.intentional_stop`, `TransitionSource::Driver`, `ServiceLifecycle`'s five action-emitting sites) — none needed a direct scenario of their own (REUSE UNCHANGED means their existing test coverage already defends them); their INTERACTION with the new reclamation class is what S-VM-26/27/29 exercise.

No new ADR or DESIGN amendment is required by this DISTILL pass.

---

## Expected RED state at DELIVER handoff

Fifteen scaffolds land RED (verified — see DWD-06 evidence block). No downstream compile fallout was produced by this pass, because every scaffold is a self-contained placeholder function in a NEW file — none of the five compiler-enforced enum/match sites `brief.md` §105a.9 names were touched (deliberately, per DWD-06 reason 2). DELIVER's Slice 01 RED phase will produce exactly that fallout when it adds `WorkloadDriver::Vm`, the `DriverRegistry` field, and the `VmReclamation` reconciler variant — and per `.claude/rules/testing.md` § "Downstream fallout on pre-existing tests is expected and correct," that is the correct shape for that step, not a defect of this DISTILL pass.

What is RED, and why:

1. **Four new test files across two crates compile and pass with the `#[should_panic(expected = "RED scaffold")]` shape.** No production code was touched.
2. **`crates/overdrive-cli/tests/integration.rs` and `crates/overdrive-core/tests/acceptance.rs` each gained one `mod` entry** (`vm_walking_skeleton`; `vm_config_pure_functions`, `vm_reclamation_plan_purity`, `vm_spec_driver_table_dispatch` respectively) — mechanical, zero fallout.
3. **Everything else in `test-scenarios.md` (59 of 74 scenarios) has no Rust artifact yet.** This is the DWD-06-scoped deferral, not an oversight — each has an exact crate/file destination already decided (DWD-04), and DELIVER scaffolds it at the start of the relevant slice's own RED phase.

### DWD-08: Peer review (Sentinel) — NEEDS_REVISION, fixed, not waived

`nw-acceptance-designer-reviewer` reviewed `test-scenarios.md`,
`wave-decisions.md`, the four scaffold files, and their entrypoint wiring
against its 9 critique dimensions. Verdict: **NEEDS_REVISION** — one
blocker, two high findings. Both fixed in this same pass; none waived.

**BLOCKER — Contract Shape Classification (mandate 14, 2026-05-15) missing
from all 74 scenarios.** Every scenario now carries a
`@contract-shape:<pure-function|bounded-change|unbounded-preservation>`
tag. Classification method: `pure-function` where the driving port is a
provably side-effect-free function (11 scenarios — the `VmConfig`/
`plan_reclamation`/`SupervisionSet` pure functions plus the `JobEnvelope`
V1 roundtrip); `unbounded-preservation` where the assertion spans an
open/non-enumerable surface — "leaks nothing," "survives every tick,"
"behaves exactly as before," "does not widen what X can reach," "nothing
left behind on any path" (6 scenarios: S-VM-14, 24, 52, 57, 61, 68);
`bounded-change` everywhere else (57 scenarios — a specific, nameable
resource/row/field transition with a closed complement, e.g. S-VM-02's
"exit code is 7, the VMM's own 0 appears nowhere" or S-VM-25's
"byte-unchanged row, every named field").

**HIGH — 2-scenario undercount in the "72 total" claim.** Caught as a
side effect of the tagging pass: a mechanical recount (every line
starting `**Tags**:`) found **74** scenarios, not 72. `test-scenarios.md`'s
§ Error / Edge Path Coverage, § Self-Review Checklist, this file, and
`feature-delta.md`'s DISTILL section were all corrected to the mechanically
recomputed figures (18 `@happy_path` / 32 `@error_path` / 12 `@edge_case` /
12 pure-`@property`-no-path-tag / 17 `@property` total / 74 scenarios;
error+edge coverage 44/74 ≈ 59%, still well above the 40% target).

**HIGH — deferred-scaffold list needs a forward-reference table for
DELIVER.** Already satisfied by DWD-04 above (which the reviewer had not
yet cross-referenced against DWD-06 when it filed the finding) — every
one of the 59 deferred scenarios already has an exact crate/file
destination committed there. No further action; noted so the finding is
recorded as addressed-by-existing-content rather than silently dropped.

**Self-caught during the fix pass (not from the reviewer, recorded for
completeness)**: S-VM-90/91 (the `vmm_equivalence.rs` / `vm_host_state_
equivalence.rs` adapter-contract tests) were tagged `@property`, which
Mandate 9 reserves for generated-input PBT at layers 1-2. Both are FIXED,
hand-enumerated call sequences at layer 3 — retagged `@example`, count
tables corrected (17 → 15 `@property`).

**MEDIUM (informational, no action needed)**: S-VM-10's deferral already
carries the explicit "DELIVER must Read the existing file before the
six-step procedure" instruction (DWD-06 point 5) — the reviewer's
recommendation was already satisfied verbatim.

Not re-dispatched for a second review pass — both structural findings
(contract-shape tags, the recount) are objectively fixed and mechanically
verifiable by grep, and the two "high" process findings were already
discharged by existing content the reviewer's own dispatch did not carry
full context for (it reviewed the two `.md` files independently, not
cross-referenced against each other's line numbers).

---

## Upstream Issues

None discovered. Every acceptance criterion in feature-delta.md's nine user stories, plus every one of Morgan's twelve numbered DISTILL handoff items and Hera's four DD-1-trap scenarios, is testable as written — the DESIGN dispatches were unusually explicit about pinning exact signatures, mutation targets, and even the specific two-shape structure of the trickiest AC (US-VM-9's byte-unchanged assertion, S-VM-25 shapes (a)/(b)). No back-propagation to DISCUSS or DESIGN is required from this DISTILL pass.

One item is flagged for the user's awareness, not as a blocker: DESIGN's own Deferrals sections (D-1…D-6, H-1…H-4, M-1…M-5) are all still open per the feature-delta.md's own record — this DISTILL does not re-litigate any of them (they are DESIGN/DEVOPS-scope, not DISTILL-scope), but M-1 (`reserve_bytes` ships as a RED scaffold and is a hard DELIVER dependency) and D-3 (unresolved by design — a cgroup OOM ships as `signal: 9` on the class most likely to cause one, mitigated in reduced form by the D-3 fold-in / `CgroupAccounting`) are directly load-bearing for S-VM-19/20 and worth DELIVER re-reading before implementing those two scenarios specifically.

**Three items were BLOCKED on the concurrent DESIGN pass / a user scoping
decision, added by the prior remediation (DWD-11) and DWD-12. All three
rulings have now landed — see DWD-12 (S-VM-65, the S-VM-13/S-VM-51
injection seam) and DWD-13 (S-VM-67) below for the finding-by-finding
dispositions. Zero open items remain.**

Resolved (kept here for the audit trail; see DWD-12 / DWD-13 for the full
ruling text):

1. ~~S-VM-65's `TransitionReason` variant for a mid-run storage-daemon
   death.~~ **RESOLVED** — ADR-0083 §D5 gained row 14,
   `TransitionReason::VmStorageDaemonDied { socket: String, exit_code:
   Option<i32>, signal: Option<u8> }` (Slice 04), checked ahead of
   `ExitKind` entirely in `exit_observer::handle_exit_event`.
2. ~~The injection seam for `SimVmm` into the production `overdrive serve`
   composition root, used by S-VM-13, S-VM-51 (and originally hoped to
   cover S-VM-67).~~ **RESOLVED for S-VM-13/S-VM-51** — ADR-0083 §D8,
   `ServerConfig.vmm_override`, `#[cfg(feature = "integration-tests")]`-
   gated, shaped after `mtls_identity_override` (a whole-port swap), not
   `dataplane_override` (a whole-subsystem gate, rejected by name in §A10).
   Never covered S-VM-67 — that scenario sits behind a different port
   entirely (virtiofsd sits outside `Vmm`); see item 3.
3. ~~S-VM-67's storage-daemon-sandbox injection seam / scoping decision.~~
   **RESOLVED by explicit user ruling (DWD-13) — path (b).** `[D8d]`'s
   `--sandbox=namespace`-unavailable case is verified at the
   **launch-argument construction layer** instead of through a real
   `overdrive serve` — the same enforcement tier ADR-0082 §D2.1 already
   uses for `image_type=raw` (private fields, one rendering site, a pure
   unit test on the rendered value). **This feature mints no
   storage-daemon supervision port** (path (a) explicitly not taken).
   `test-scenarios.md` S-VM-67 rewritten accordingly (`@tier1`/
   `@in-memory`, pure-function driving port); the deploy-level
   fail-closed claim is recorded, explicitly, as an undischarged Tier-3
   property of Slice 04 — not proven by this feature, not silently
   assumed either.

---

## DWD-11: Adversarial-review remediation (2026-08-11) — finding-by-finding disposition

Two independent fable reviewers (Sentinel, `nw-acceptance-designer-reviewer`,
structural/BDD focus; Atlas, an ad-hoc adversarial pass) both returned
`needs_revision` against the DISTILL artifacts committed under DWD-08.
This entry records what was fixed, what was deliberately deferred (with
reasons), and the one item genuinely BLOCKED on a concurrent DESIGN pass.
**Scenario count: 74 → 87** (+13; error/edge ratio 59%→60%, mechanically
recomputed — see DWD-05). **Zero dangling `S-VM-N` references** anywhere in
`test-scenarios.md`, `feature-delta.md`, or this file (re-verified via
`grep -oE 'S-VM-[0-9]+' | sort -u` cross-checked against every `#### S-VM-N`
header, both directions, after every edit).

### BLOCKER

- **S-VM-88/S-VM-89 phantom references; the third §105a.11 ESR invariant
  had no scenario ID at all.** FIXED — S-VM-87 (`VmReclamationIdempotentSteadyState`),
  S-VM-88 (`VmReclamationConverges`), S-VM-89 (`EndingInFlightIsNeverReclaimed`)
  defined as real scenarios under a new AC-20, added to the Driving Ports
  table, the Adapter Coverage Table, and the Error/Edge and contract-shape
  counts.

### HIGH — systemic (the NEW-1 pins)

- **The catalogue was systematically thin exactly where DESIGN's iteration-2
  remediation added the DD-1(b.i) kill-authorising mechanics.** FIXED — four
  new scenarios under a new AC-19 (S-VM-77 abandonment boundary, S-VM-78
  hydration read order, S-VM-79 write-time terminality guard, S-VM-80 P2
  as a property directly over `VmReclamation`), closing Sentinel's own
  MEDIUM-1 finding at the same time.

### HIGH — the rest

- **The fourth evaluation (`svid_lifecycle`/`DropSvid`).** FIXED — S-VM-81.
- **S-VM-12 vs S-VM-35 contradiction.** FIXED — S-VM-35 rewritten to the
  TOCTOU window (CH present at boot, removed before an individual start);
  the original precondition (CH absent at boot) is S-VM-12's, exclusively.
- **`CgroupAccounting` had no equivalence scenario.** FIXED — S-VM-93,
  covering read-once semantics and ADR-0082 §D8's three-row probe fault
  table.
- **S-VM-13's envelope claim was false for the non-reflink class.** FIXED —
  S-VM-13 narrowed to the capability-flag classes (Landlock flag/LSM
  absence, KVM unreachability) plus the run-root class, where the "no
  genuinely lying host" claim IS true; S-VM-75 added as the real
  non-reflink substrate arm. S-VM-51/S-VM-67 left as-is (defensibly
  injected, per the finding) but given the same blocked-injection-seam
  note as S-VM-13.
- **S-VM-49 contradicted S-VM-53.** FIXED — reworded to the full correct
  ruleset (CH's auto-derived grants PLUS the one directory read-write
  grant), dropping the false "ONLY" claim.
- **ADR-0082 §D4's named enforcement vehicle (`VmDriver`-level acceptance
  case against `SimVmm`) was forbidden by the Driving Ports table.** FIXED
  — S-VM-76 added under a new AC-18; the Driving Ports table's
  `VmDriver::start`/`stop` row reclassified from "never invoked directly"
  to a documented, justified carve-out citing ADR-0082 §D4 by name.
- **`MtlsInterceptWorker` gating (§D2a(c)) — only the fail-closed arm was
  covered.** FIXED — S-VM-74, asserting no intercept state exists for a
  gated-off VM allocation.
- **The `FICLONE` per-launch clone (C-1, ADR-0082 §D5, brief §107) — only
  the boot probe was covered.** FIXED — S-VM-94, adapter-level, real
  non-reflink target, typed errno never a silent full-copy fallback.

### MEDIUM / LOW

- **S-VM-26 named the wrong variant.** FIXED — corrected to
  `TerminalCondition::Failed { exit_code: Some(0) }`, per `brief.md`
  §104/§105a.10.
- **S-VM-20's mutation obligation was misplaced.** FIXED — `@mandatory:mutation_target`
  removed from S-VM-20 (a `todo!()` has nothing to mutate); the Then
  restated as machine-checkable bounds (a range, not a prose "is derived
  from"); the mutation obligation restated as a DELIVER-step gate in the
  scenario's own crafter note, citing `brief.md` §113's own wording.
- **No AC for any `dst-lint` clause.** RECORDED AS AN EXPLICIT DECISION,
  not a new scenario shape — see DWD-09. Each of the five clauses is an
  `xtask/src/dst_lint.rs` unit test (following the existing
  `CrashObservabilityStructLiteral` precedent), mapped 1:1 to the DELIVER
  step that introduces the guarded type and to the behavioral scenario it
  complements; pointer notes added to S-VM-08, 16, 18, 53, 77.
- **DWD-06's scaffold accounting was false.** FIXED — corrected: 12 of 20
  Slice-01 IDs are scaffolded (not "every scenario in Slice 01"); the
  deferred list now includes S-VM-11 and S-VM-12, which the original list
  omitted entirely.
- **DWD-03 said four policy rows; six landed.** FIXED — corrected to six
  (1 driving + 3 driven-internal + 2 driven-external).
- **`test-scenarios.md:1364` still said "subprocess-driven scenario."**
  FIXED — reworded to "direct-CLI-handler-driven scenario against a real
  in-process `overdrive serve`," matching DWD-07's no-subprocess
  correction.
- **Non-assertable Thens (S-VM-20, S-VM-08, S-VM-44).** FIXED — the
  workspace-negative clauses ("no code path in the workspace can produce…",
  "this is the ONLY path in the workspace…") moved out of `Then` into
  crafter notes / mutation-target framing; the `Then` in each scenario now
  contains only port-observable assertions.
- **Kernel matrix unowned.** RECORDED AS A DWD NOTE (DWD-10) rather than a
  per-scenario tag — retagging ~55 Tier-3 scenarios would not change which
  kernel actually runs them; ownership assigned to the existing CI Tier-3
  LVH lane, which already runs both matrix legs.
- **S-VM-37 named no `TransitionReason` for "unclassified."** FIXED —
  pinned to the EXISTING `TransitionReason::DriverInternalError { detail }`
  fallthrough variant (`transition_reason.rs`); no new variant needed.

### Deliberately NOT re-litigated (per the user's explicit ruling)

- **The 59-of-74 (now 72-of-87) scaffold deferral.** Sentinel's BLOCKER on
  this point is SETTLED, not fixed — see DWD-06a. The user ruled
  `.claude/rules/testing.md` (a checked-in project rule assigning AT-to-
  Rust translation to the crafter) governs over the generic `nw-distill`
  skill's ADR-025 statement, per CLAUDE.md's own project-instructions-
  override-defaults precedence. Ownership answered: DELIVER's
  `nw-software-crafter` authors each slice's scaffolds; DELIVER's
  `nw-software-crafter-reviewer` reviews them.

### Concurrent DESIGN dependency — not guessed

- **S-VM-65's hedge** ("`VmGuestMountFailed`'s sibling variant, or a
  distinct mid-run variant") REMOVED. No variant exists for a mid-run
  storage-daemon death in ADR-0083 §D5's table. Marked BLOCKED on the
  concurrent ruling in both `test-scenarios.md` and the Upstream Issues
  section above.
- **S-VM-13/51/67's injection seam** into the production composition root
  MARKED BLOCKED, not guessed. ADR-0083 §D2 shows only production
  discovery; DISTILL does not invent a `vmm_override`-shaped mechanism.

---

## DWD-12: Concurrent DESIGN pass reconciliation (2026-08-11) — two of the
three BLOCKED items unblocked; one stays blocked, precisely

The concurrent DESIGN pass DWD-11 deferred to has now ruled (feature-delta.md
`## Wave: DESIGN — application / component scope (Morgan, 2026-08-11)`
changelog, "two DISTILL-surfaced gap closures"; ADR-0083 gained §D5 row 14
and §D8; ADR-0082's Status header and §D5 fault-injection table gained a
cross-reference amendment). This entry unblocks S-VM-65, S-VM-13, and
S-VM-51 in `test-scenarios.md`, and refines — without unblocking —
S-VM-67's note. No scenario was added or removed; the scenario count stays
**87** (mechanically re-verified: `grep -c '^#### S-VM-' distill/test-scenarios.md`
and `grep -c '^\*\*Tags\*\*:' distill/test-scenarios.md` both return 87).

**Ruling 1 — S-VM-65's mid-run storage-daemon death (RESOLVED).** ADR-0083
§D5 gained row 14: `TransitionReason::VmStorageDaemonDied { socket: String,
exit_code: Option<i32>, signal: Option<u8> }`, scoped to Slice 04 — a
**distinct** variant, not a reuse of `VmGuestMountFailed` (row 10, which
stays scoped to the guest-reported start-time mount failure). Cause count
is now **fourteen**, not twelve or thirteen. The fact is carried on a new
additive `ExitEvent.storage_daemon_died` field (mirroring ADR-0082 §D8's
`oom` field), and `exit_observer::handle_exit_event` checks it **ahead of
`ExitKind` entirely** — not nested inside the `Crashed` arm the way
`VmOutOfMemory` (row 13) is. `test-scenarios.md`'s S-VM-65 now carries two
scenario shapes: (a) the daemon dies and the guest never resolves an
outcome of its own — the trivial case any implementation gets right; (b)
the daemon dies but the guest's own command then exits 0 and self-reports
`EXIT 0` over vsock — the discriminating case that fails if the precedence
check is nested inside `Crashed` instead of running first, since a guest
that self-reports success after its share died would otherwise resolve
`ExitKind::CleanExit` before the daemon-death fact is ever consulted,
silently reproducing `VmGuestMountFailed`'s composite-lie defect one
execution phase later — the exact failure US-VM-9 Scenario 2 exists to
prevent. Disposition is `StoppedBy::Process`, never `PlatformReclaimed`
(DD-3's two-axis rule — Cause and Disposition are orthogonal).

**Ruling 2 — the `SimVmm` injection seam for S-VM-13 and S-VM-51
(RESOLVED).** ADR-0083 gained §D8: `ServerConfig.vmm_override:
Option<Arc<dyn overdrive_core::traits::vmm::Vmm>>`, `#[cfg(feature =
"integration-tests")]`-gated on both the declaration and its one use site.
Shaped after the already-shipped `mtls_identity_override` (whole-**port**
substitution) deliberately, NOT after `dataplane_override` (whole-
**subsystem** gate) — §A10 records the latter as considered and rejected
by name, being the exact GH #248 / ADR-0074 shape this seam avoids on
purpose. The states this seam injects (`ReflinkUnsupported`,
`LandlockFlagAbsent`, `LandlockLsmAbsent`, `KvmUnreachable`,
`RunDirUnusable`) are ADR-0082 §D5's own catalogued, **production-
reachable** substrate lies — not states only the seam can produce.
`.probe()` still runs unconditionally against whichever adapter is bound,
so Earned Trust is never bypassed. `test-scenarios.md`'s S-VM-13 and
S-VM-51 crafter notes now name the seam and gating exactly; the `@real-io`
Adapter Coverage Table's `Vmm (SimVmm)` row and the Self-Review Checklist
item 4 were corrected to match (S-VM-67 removed from both — it is not
covered by this seam, see Ruling 3).

**Ruling 3 — S-VM-67 stays BLOCKED, precisely: a scoping decision, not a
missing name.** The architect ruled explicitly (ADR-0083 §D8, "What the
seam does NOT reach — S-VM-67, stated plainly rather than glossed") that
S-VM-67 is **not reachable** through the §D8 seam: `Vmm::create` spawns
"ONE confined hypervisor process" (ADR-0082 §D1), `VmConfig` carries no
volume field, and no `Vmm` method sits downstream of virtiofsd's sandbox
check — `virtiofsd` is a sidecar `VmDriver` spawns and supervises directly,
outside the `Vmm` port entirely (system constraint 9 / US-VM-9). Two
honest paths were named (a future Slice-04 port mirroring §D8's
probe/fault-injection-table shape, with its own `ServerConfig` override
field; or asserting the case at a level narrower than a real `overdrive
serve` — a pure unit test over spawn-argument construction plus a
Tier-2-shaped fail-closed assertion) and **neither was chosen** — that
decision is with the user. `test-scenarios.md`'s S-VM-67 crafter note is
updated to state this precisely (it was previously worded identically to
S-VM-13/S-VM-51's now-resolved blocker, which conflated "the mechanism is
unpinned" with the true state, "the mechanism does not reach this port at
all"); its `**BLOCKED**` marker stays in place. No supervision port is
invented by this entry, per CLAUDE.md § "Implement to the design."

**Upstream Issues, updated.** The section above now reflects exactly **one**
open item (S-VM-67's scoping decision) instead of the prior two
BLOCKED-items/three-blocked-scenarios framing (S-VM-65 as item 1; S-VM-13,
S-VM-51, S-VM-67 grouped under item 2). Both resolved items are kept,
struck through, for the audit trail.

**Files touched by this entry**: `distill/test-scenarios.md` (S-VM-13,
S-VM-51, S-VM-65, S-VM-67 crafter notes + gherkin; the `Vmm (SimVmm)`
Adapter Coverage Table row; the US-VM-9 AC-to-Scenario Traceability row;
Self-Review Checklist item 4), `distill/wave-decisions.md` (this entry;
Upstream Issues section). `feature-delta.md`'s `## Wave: DISTILL` section
was reviewed and needs no edit — it does not enumerate the BLOCKED items by
scenario ID, only the aggregate 87-scenario count and tag taxonomy, both
unchanged. No ADR, `brief.md`, or Rust file was touched by this entry — per
scope, those are the concurrent DESIGN pass's and DELIVER's, respectively.
No GitHub issue created or referenced; #259–#263 remain the only real
numbers in scope, #264 closed, none newly cited here.

---

## DWD-13: S-VM-67 unblocked by explicit user ruling (2026-08-11) — path
(b), the launch-argument construction layer, mirroring S-VM-16

The user has ruled on the one item DWD-12 left open. **Path (b)**: assert
`[D8d]`'s `--sandbox=namespace`-unavailable case below `overdrive serve`,
at the launch-argument construction layer — the same enforcement tier
ADR-0082 §D2.1 already uses for `image_type=raw` (private fields, exactly
one rendering site, a pure unit test on the rendered value). **No
storage-daemon supervision port is minted by this feature** — path (a) is
explicitly not taken. The concurrent DESIGN pass already landed the ADR
amendments this ruling drove (ADR-0082 §D2.1's cross-reference amendment;
ADR-0083 §D8's closing amendment, including the "Negative, and stated"
bullet) before this DISTILL pass started; this entry reconciles
`test-scenarios.md` and this file against those amendments, and records
the DISTILL-side reasoning for each downstream change. No scenario was
added or removed; the count stays **87** (mechanically re-verified:
`grep -c '^#### S-VM-'` and `grep -c '^\*\*Tags\*\*:'` both return 87 after
every edit in this pass).

**S-VM-67's new shape and tier.** Rewritten from a Tier-3, `overdrive
deploy`-driven, `@error_path`/`@tier3`/`@real-io` scenario asserting a
deploy-level `Failed` outcome, to a **pure-function, `@tier1`/`@in-memory`
property scenario** asserting on the storage daemon's launch-argument
rendering site directly — `@contract-shape:pure-function`, `@property`,
`@error_path` retained (the rejection contract survives the tier change;
S-VM-17's `KernelImage::validate` is the exact precedent for a pure
function that is BOTH `@property` and `@error_path`), `@mandatory:
mutation_target` retained, `@correction:D8d` retained. Title changed from
"A host that cannot sandbox the storage daemon refuses the workload" (a
claim the new scenario does not make) to "The storage daemon's launch
argument never carries a weaker sandbox than `--sandbox=namespace`" (the
claim it does make). Full text: `distill/test-scenarios.md` S-VM-67.

**No runtime-half Tier-3 scenario was added.** Considered and rejected,
for reasons stated in S-VM-67's own crafter note and repeated here for the
audit trail: (i) the user's ruling is explicit that this feature mints no
storage-daemon supervision port, so there is no seam a Tier-3 scenario
could inject an unavailable-capability fault through (`ServerConfig.
vmm_override` does not reach here — ADR-0083 §D8, "What the seam does NOT
reach"); (ii) the single-kernel Lima test envelope genuinely supports
`--sandbox=namespace` (`spike/findings.md` line 362), so there is no
real, un-injected lying host to exercise the failure against either;
(iii) building either a port or a seam now, absent a design decision that
mints one, is exactly the "invent API surface past the design" move
CLAUDE.md and ADR-0083 §D8 both forbid. If Slice 04's own future DESIGN
mints a storage-daemon supervision port on its own merits, the Tier-3
fail-closed scenario is that slice's own DISTILL addition, not
retrofitted onto this feature's catalogue.

**The boundary statement, written into S-VM-67's crafter note verbatim
(summarised here).** The rewritten scenario's `Then` proves only what
argument the rendering function constructs — that rendering
`--sandbox=namespace` is representable at exactly one site, with no
second call site and no field that could carry `chroot`. It does **not**
prove a *running* `virtiofsd` enforces the flag, nor that the *platform*
turns a host's genuine incapacity into a `Failed` allocation end-to-end.
Both halves are named, explicitly, as an **undischarged Tier-3 property
of Slice 04** — this is ADR-0083 §D8's own honesty, carried into the
DISTILL artifact rather than softened. This scenario's `Then` must never
be cited as proof of the runtime posture.

**Sibling references checked and corrected** (the same three DWD-12 fixed
for the S-VM-13/S-VM-51 injection-seam resolution, re-verified here
against the new S-VM-67 resolution rather than merely against its old
BLOCKED state — plus two more this pass found while checking):

1. **`@real-io` Adapter Coverage Table, `Vmm (SimVmm)` row** — already
   correct from DWD-12 (never listed S-VM-67; the seam never covered it).
   No change needed.
2. **`@real-io` Adapter Coverage Table, "virtiofsd storage daemon (real
   supervised host process)" row** — DWD-12 did not touch this row
   (S-VM-67's content was still Tier-3/real-io at the time). **Now
   corrected**: S-VM-67 removed from the covered-by list — the rewritten
   scenario touches no real virtiofsd process. The adapter keeps 7 other
   `@real-io` scenarios (S-VM-55, 56, 59, 64, 65, 66, 68); zero "NO —
   MISSING" rows introduced.
3. **US-VM-9 AC-to-Scenario Traceability row** — reworded from "S-VM-67
   remains BLOCKED on a scoping decision (ADR-0083 §D8)" to "RESOLVED by
   explicit user ruling (DWD-13) — rewritten to the pure
   launch-argument-construction layer... the deploy-level fail-closed
   claim stays an undischarged Tier-3 property of Slice 04."
4. **Self-Review Checklist item 4** — reworded from "S-VM-67 stays
   BLOCKED on a scoping decision, not resolved by this seam" to "RESOLVED,
   not by this seam: explicit user ruling (DWD-13) moves S-VM-67 to the
   pure launch-argument-construction layer instead."
5. **NEW — top-of-file "Driving Ports" table, `overdrive deploy` row**
   (not checked by DWD-12; it predates that entry's scope). The range
   `S-VM-33…68` silently included S-VM-67 among the Tier-3 CLI-driven
   scenarios. **Corrected** to `S-VM-33…66, S-VM-68` with an explicit
   exclusion note, and a new row added for the storage-daemon
   launch-argument rendering site, explicitly marked "exact type NOT yet
   pinned by any ADR."
6. **NEW — `distill/wave-decisions.md` DWD-04's crate-placement table**
   (this file, not `test-scenarios.md`; same silent-range issue as #5).
   "S-VM-01…05, 11…15, 33…68" corrected to "...33…66, 68" with a note
   pointing at this entry.
7. **NEW — Error / Edge Path Coverage counts.** `@property` moves 20 → 21
   (S-VM-67 gained the tag); `@error_path` stays 40 (S-VM-67 keeps it);
   `@tier3`/`@real-io` each drop by one, `@tier1`/`@in-memory` each gain
   one; total scenario count unchanged at 87; error+edge coverage
   unchanged at 52/87 ≈ 60%.

**Files touched by this entry**: `distill/test-scenarios.md` (S-VM-67's
full rewrite; the top-of-file Driving Ports table; the `@real-io` Adapter
Coverage Table's virtiofsd row; the US-VM-9 AC-to-Scenario Traceability
row; Self-Review Checklist item 4; the Error/Edge Path Coverage counts),
`distill/wave-decisions.md` (this entry; DWD-04's crate-placement table;
the Upstream Issues section, below). No ADR, `brief.md`, or Rust file was
touched by this entry — those already landed via the concurrent DESIGN
pass (ADR-0082, ADR-0083) before this DISTILL pass started; DELIVER's
Slice 04 RED phase is the remaining consumer. No GitHub issue created or
referenced; #259–#263 remain the only real numbers in scope, #264 closed,
none newly cited here. No commit made by this pass.

---

## DWD-14: AC-09 completeness gap closed — S-VM-41, `VmKernelFormatUnsupported` (2026-08-11)

A fable reviewer cross-checking the concurrent `deliver/roadmap.json` pass
against ADR-0083 §D5 found that Slice-02 step 03-01's criteria enumerate
**four** Cause variants (`VmKernelNotFound`, `VmRootfsNotFound`,
`VmHypervisorAbsent`, `VmBootDeadlineExceeded`) where the ADR's own §D5
table pins **five** for Slice 02 — row 5 is `VmKernelFormatUnsupported {
path, arch, detail }`, the C-7 correction (`slice-02.md`'s own
`superseded-by-DESIGN` block: *"The count is five, not four"*). Verified
directly before acting: `grep -rn "VmKernelFormatUnsupported"
distill/` returned nothing outside `slices/slice-02-boot-failure-
vocabulary.md`'s own prose — the variant genuinely had **zero**
`test-scenarios.md` entry among the original 87. AC-09's own five
scenarios (S-VM-33…37) covered exactly the four *new* variants plus the
unclassified `DriverInternalError` fallthrough (S-VM-37, which reuses an
EXISTING variant and was never meant to stand in for row 5) — a genuine
gap, not a mis-tag.

**Fix**: S-VM-41 added (`distill/test-scenarios.md`, physically placed
after S-VM-37's crafter notes, inside AC-09 — content-grouped, per this
file's own established convention of placing later-added scenarios by AC
rather than by ID sort order, e.g. S-VM-74/76/77…81/87…89 in Slice 01).
**Explicitly scoped to the classification join, not a duplicate of
S-VM-17.** S-VM-17 already proves `KernelImage::validate` is pure and
rejects the bad magic bytes before any hypervisor process is spawned
(ADR-0082 §D2.4), covering the identical aarch64-UKI-wrapper artifact at
the function boundary. S-VM-41 proves the layer above it:
`classify_driver_failure`'s VM arm maps the resulting `KernelFormatError`
onto `TransitionReason::VmKernelFormatUnsupported` (ADR-0083 §D5 row 5),
observed through `overdrive deploy` + `overdrive workload describe`
exactly like its four AC-09 siblings, and asserts on the OPERATOR-VISIBLE
wording — the reported cause reads as a format problem, never CH's
misleading `UefiTooBig`/size-cap framing — not merely "some error
occurred." A vacuous version of this scenario (asserting only "the
deploy fails") would have passed against the pre-fix misleading surface
and proven nothing; this is the closed-world-effect trap this feature's
own `@contract-shape:` mandate exists to catch (Mandate 14 tag:
`@contract-shape:bounded-change`, matching S-VM-33…37's shape — a
specific, nameable field transition on a specific allocation, not an
open-ended claim).

**Tier and tags**: `@contract-shape:bounded-change` `@error_path`
`@ac-09` `@tier3` `@real-io` `@correction:C-7` — identical tier/tag family
to S-VM-33…36 (Tier-3, CLI-driven, real `overdrive serve`, no port
injection needed since the failure is reached by a genuinely-invalid
on-disk artifact, not a simulated fault).

**Placement — DWD-04 extended.** DWD-04's first row already names the
Tier-3 CLI-driven range `S-VM-01…05, 11…15, 33…66, 68` — a numeric span,
not an exhaustive enumeration (the range already silently absorbed gaps
such as S-VM-54 before any scenario existed at that ID). S-VM-41 (33–66
span) is therefore already covered by that row's existing text; no range
edit was needed. Recorded explicitly here for auditability, and in
`test-scenarios.md` S-VM-41's own crafter note: **placed at
`crates/overdrive-cli/tests/integration/vm_boot_failure_vocabulary.rs`**,
the same file `deliver/roadmap.json` step 03-01/03-02 already use for
S-VM-33…37 — DELIVER's RED phase scaffolds it there alongside its
siblings, per this feature's per-slice deferred-scaffold discipline
(DWD-06).

**Scenario ID choice.** `S-VM-41` — the lowest genuinely-unused gap in
the file's existing ID sequence (mechanically confirmed:
`grep -oE '^#### S-VM-[0-9]+' test-scenarios.md` showed no `S-VM-41`
anywhere before this entry; the file already contains gaps at 41, 54,
82–86, consistent with this project's established practice of assigning
each newly-discovered scenario the next free ID rather than renumbering
the catalogue — the same discipline DWD-11/DWD-12/DWD-13 followed when
they added S-VM-74…81/87…89/93/94 without renumbering anything). No
scenario was renumbered or removed by this entry.

**Every count in `test-scenarios.md` re-verified mechanically after the
addition** (`grep -c '^#### S-VM-'` and `grep -c '^\*\*Tags\*\*:'`, both
**88**; per-tag counts via `grep '^\*\*Tags\*\*:' | grep -c '@<tag>'`):
`@error_path` 40 → 41; `@contract-shape:bounded-change` 65 → 66;
`@property` unchanged at 21 (S-VM-41 is example-shaped, not a property);
error+edge coverage 52/87 ≈ 60% → 53/88 ≈ 60% (unchanged ratio). **The
mechanical recount also surfaced a pre-existing, unrelated off-by-one**
in Self-Review Checklist item 13's `@contract-shape:pure-function` /
`@contract-shape:bounded-change` split: it read "11 pure-function … 66
bounded-change," but a direct listing showed 12 pure-function tags were
already present before this pass (65 bounded-change, not 66) — the two
wrong numbers happened to still sum to 87, which is exactly how the drift
went undetected. Corrected in place in `test-scenarios.md` (Self-Review
Checklist item 13) as an incidental fix while this entry's own mechanical
verification was already running; not a consequence of adding S-VM-41,
and no other count in the file was found to be similarly stale.

**Scenario updated in three places besides its own entry**: the KPI
Traceability K3 row (`S-VM-33…37` → `S-VM-33…37, S-VM-41`, making the
row's pre-existing "5 distinct `TransitionReason` variants at Slice 02"
claim accurate for the first time — it had said "5" while only 4 new
variants + 1 fallthrough were actually covered); the AC-to-Scenario
Traceability US-VM-2 row (same ID addition, count text updated); the
Error / Edge Path Coverage narrative + table; Self-Review Checklist items
8/13/15 (item 15 newly added, recording this gap-closure explicitly).

**Files touched by this entry**: `distill/test-scenarios.md` (new S-VM-41
scenario + crafter note; Driving Ports table needed no edit — the
`overdrive deploy` row's `S-VM-33…66` span already covers it; KPI
Traceability; AC-to-Scenario Traceability; Error/Edge Path Coverage;
Self-Review Checklist items 8/13/15), `distill/wave-decisions.md` (this
entry; Changelog, below). **`deliver/roadmap.json` was NOT touched** —
per this dispatch's scope, the concurrent roadmap pass owns it and cites
S-VM-41 in its own Slice-02 step using the ID recorded here.
`docs/product/architecture/adr-0083-*.md`, `adr-0082-*.md`, and
`brief.md` were NOT touched — row 5 and D2.4 already existed there; this
entry closes a DISTILL-side test-coverage gap, not a design gap. No
GitHub issue created or referenced; #259–#263 remain the only real
numbers in scope, #264 closed, none newly cited here. No commit made by
this pass.

---

## DWD-15: S-VM-06/07/62 driving-port prose corrected to match DWD-04's parse-boundary placement (2026-08-11, iteration-2 review remediation, LOW)

An iteration-2 review found S-VM-06, S-VM-07, and S-VM-62's per-scenario
`**Driving port**:` lines read `overdrive deploy` (in-process CLI
handler, no subprocess) — while DWD-04's crate-placement table (the
"Parse-boundary rejections (S-VM-06/07/62)" row) places all three at
`overdrive-core`'s `tests/acceptance/vm_spec_driver_table_dispatch.rs`,
default lane, rationale "in-process TOML deserializer, no subprocess and
no real VMM needed." The two cannot both be literally true: `overdrive
deploy` is `overdrive-cli`'s command handler, and `overdrive-core`
cannot dev-depend on `overdrive-cli` (verified — `overdrive-core`'s
`Cargo.toml` carries no `overdrive-cli`/`overdrive-control-plane`
dependency; the dependency direction runs the other way, per
`development.md` § "Port-trait dependencies"). A crafter reading the
scenario's own driving-port line, not DWD-04, would try to invoke the
CLI handler from a core-lane test and find it unimportable.

**Verified before fixing, not assumed.** The lane and file were already
correct and already had ground truth: `crates/overdrive-core/tests/
acceptance/vm_spec_driver_table_dispatch.rs` exists, is one of the
fifteen DWD-06 scaffolds, is verified compiling and RED, and its own
module docstring already states the true driving port: *"Driving port:
the TOML spec parser (in-process, no subprocess, no `overdrive serve`
needed — a pure parse-boundary rejection)."* The exact function is
pinned by ADR-0083 (brief.md's Reuse Analysis row 18):
`WorkloadSpecInput::from_toml_str` (`crates/overdrive-core/src/
aggregate/workload_spec.rs:710`) — the single parse-boundary function
whose `!has_exec` branch (`:743-745`) is being replaced by the
`MissingDriverSection`/`MultipleDriverSections` dispatch. S-VM-62 (the
`[[vm.volume]]` unknown-key rejection) is grouped with S-VM-06/07 in the
SAME DWD-04 row and file — its rejection surfaces through the same
top-level TOML deserialization boundary (a nested `deny_unknown_fields`
struct failing during the same `from_toml_str` call), so the same
driving-port wording applies.

**Fix**: all three `**Driving port**:` lines rewritten to
`` `WorkloadSpecInput::from_toml_str()` (pure function — in-process TOML
parse boundary, no subprocess, no `overdrive serve` needed) ``, matching
the convention this file already uses for other pure-function driving
ports (`` `VmConfinement::seccomp_arg()` (pure function) `` etc.). Only
the metadata line changed — Gherkin bodies, tags, tier, and ACs are
byte-identical to before (S-VM-06's `When` clause still narrates the
operator-facing verb "submits it via `overdrive deploy`" in business
language; that is the domain-language description of the user action,
not the driving-port mechanism, and DWD-07 already establishes this
project's CLI-verb-vs-test-mechanism distinction).

**Sweep for the same defect class.** Every scenario's `**Driving port**:`
line was cross-checked against its DWD-04 (and DWD-11-extended) crate
placement. Two adjacent findings surfaced, **neither fixed by this
entry**:

1. **S-VM-77 and S-VM-79 name driving ports that structurally cannot
   live where DWD-04 places them.** DWD-04's DWD-11 addition places
   S-VM-77…80 together at `overdrive-core` (`tests/acceptance/
   vm_reclamation_plan_purity.rs`, extended, or a new sibling file).
   S-VM-77's own driving-port line names *"the exit observer's loop body
   (`worker/exit_observer.rs:204-371`)"* and S-VM-79's names
   *"`execute_reclaim_allocation` (component-scope,
   `overdrive-control-plane`)"* — both explicitly `overdrive-control-plane`
   code (`crates/overdrive-control-plane/src/worker/exit_observer.rs`
   confirmed to exist at that path; `overdrive-control-plane` confirmed
   to depend on `overdrive-core`, never the reverse). S-VM-78 and S-VM-80
   in the same DWD-04 row are NOT affected — their driving ports
   (`VmReclamationState::hydrate_actual`, `plan_reclamation`) are
   genuinely `overdrive-core`-resident, matching DWD-04's rationale text
   ("same family as S-VM-31/32/92"). **Not fixed here** because, unlike
   S-VM-06/07/62, there is no existing scaffold to serve as ground
   truth — DWD-06 confirms S-VM-77…80 are unscaffolded — and DWD-04's own
   text already flags the exact *file* for this row as provisional
   ("DELIVER's own call once the `VmSupervision` sum type exists");
   resolving whether the *crate* cell should move to
   `overdrive-control-plane` or the driving-port prose should be
   rewritten to an `overdrive-core`-reachable equivalent is a placement
   judgment call, not a prose staleness fix, and is out of this entry's
   scope. Flagged for whoever scaffolds AC-19 in DELIVER.
2. **S-VM-09 and S-VM-19 have no DWD-04 placement at all** (an omission,
   not a contradiction — the defect class asked for). Both carry
   CLI-driven `overdrive deploy` driving-port lines consistent with
   DWD-04's Tier-3 row's *neighbors* (S-VM-01…05/11…15/33…66/68), but
   sit in the numeric gaps the row's span notation does not cover (06-10
   and 16-32 respectively) and are not named by any other DWD-04 row
   either. DWD-06's own deferred-scenario list already groups S-VM-09
   with S-VM-11…15 and S-VM-19 with the same CLI-driven family, so the
   omission looks like an oversight in DWD-04's span text rather than a
   deliberate exclusion — but this entry does not touch DWD-04's ranges
   to avoid re-scoping beyond the assigned finding.

**Files touched by this entry**: `distill/test-scenarios.md` (three
`**Driving port**:` lines, S-VM-06/07/62 only — no Gherkin, tag, tier, or
AC changed); `distill/wave-decisions.md` (this entry; Changelog, below).
`deliver/roadmap.json`, every ADR, `brief.md`, and every `.rs` file were
NOT touched — this is a DISTILL-side prose correction only, and the
roadmap pass is concurrent and out of this entry's scope. Scenario count
re-verified mechanically unchanged at **88** (`grep -c '^#### S-VM-'` and
`grep -c '^\*\*Tags\*\*:'`, both 88, matching before this entry). No
GitHub issue created; #259–#263 remain the only real numbers in scope,
#264 closed, none newly cited here. No commit made by this pass.

---

## DWD-16: The two placement gaps DWD-15 declined to resolve on the spot —
resolved (2026-08-11)

DWD-15's sweep found two gaps of the same defect class as S-VM-06/07/62
(a crafter following DWD-04 to the stated crate finds the driving-port
surface unimportable, or finds no placement at all) but correctly declined
to fix either inline, because both needed a placement judgement rather
than a prose correction. This entry makes that judgement. No scenario's
tier, tags, ACs, or Gherkin (Given/When/Then) changed. No scenario was
added, removed, or renumbered. Scenario count re-verified mechanically
unchanged at **88** (`grep -c '^#### S-VM-'` and
`grep -c '^\*\*Tags\*\*:'`, both 88, both before and after this entry).

### Gap 1 — S-VM-77/S-VM-79 vs their DWD-04 (DWD-11) `overdrive-core`
placement: contradiction, not staleness

**Verified before fixing.** `crates/overdrive-control-plane/src/worker/
exit_observer.rs` exists at exactly the path S-VM-77's driving-port line
names. `execute_reclaim_allocation` (S-VM-79's driving-port line) does not
exist yet — expected, per DWD-06's per-slice scaffold deferral — but this
crate's `action_shim/` module is the established home for every other
per-`Action`-variant executor (`action_shim/issue_svid.rs`,
`action_shim/release_service_vip.rs`,
`action_shim/register_local_backend.rs`,
`action_shim/write_service_backend_row.rs`,
`action_shim/deregister_local_backend.rs`,
`action_shim/dataplane_update_service.rs`,
`action_shim/enqueue_evaluation.rs` — all confirmed present at
`crates/overdrive-control-plane/src/action_shim/`), so
`execute_reclaim_allocation` (the `ReclaimAllocation` executor) belongs in
the same module by the crate's own existing convention. Dependency
direction confirmed both ways: `overdrive-control-plane/Cargo.toml` names
`overdrive-core.workspace = true`; `overdrive-core/Cargo.toml` carries no
`overdrive-control-plane` (or `overdrive-cli`) dependency line at all (its
three textual mentions of the string are prose comments, not `[dependencies]`
entries). So DWD-04's placement of S-VM-77/79 at `overdrive-core`
was not stale prose (the S-VM-06/07/62 shape) — it was a genuine
contradiction: the stated crate cannot compile a test that imports either
named driving port. S-VM-78 and S-VM-80, grouped in the same original row,
are unaffected — `VmReclamationState::hydrate_actual` and
`plan_reclamation` are both genuinely defined in
`crates/overdrive-core/src/reconcilers/vm_reclamation.rs`.

**Remedy chosen: move, not rewrite.** Per the dispatch's instruction not to
fabricate a core-side seam to preserve a tidy table — and because none
exists: the claim-release loop (S-VM-77) is the exit observer's own
control-flow, not a pure function `overdrive-core` could host, and the
write-time terminality guard (S-VM-79) is specifically the *executor*
half of `ReclaimAllocation` (the re-read-and-refuse guard around
`kill_scope`/`discard_artifacts`/the row write), which is
`overdrive-control-plane`'s job by this project's own reconciler/executor
split (`.claude/rules/reconcilers.md` — "An executor already driven by a
reconciler... is an action executor, not a reconciler candidate";
`plan_reclamation` computes the `Action`, `execute_reclaim_allocation`
performs it). Rewriting the driving-port prose to a core-reachable
equivalent was considered and rejected: no such equivalent exists without
either (a) inventing a new pure predicate that doesn't match what
`brief.md` §105a.3/§105a.5 actually specify (the release-on-every-arm and
write-time-guard behaviours are stated over the loop body and the
executor, not over a hypothetical pure projection of them), or (b)
weakening S-VM-77/79 to test something narrower than what they were
written to prove — the P5 NEW-1 guard against a stale-arm claim leak,
and the write-time refusal guard against a TOCTOU race. Both would be the
"fabricate a core-side seam" move the dispatch explicitly forbade.

**Fix**: DWD-04's AC-19 row split into two — the original row now covers
only S-VM-78/80 at `overdrive-core`; a new row places S-VM-77/79 at
`overdrive-control-plane`, `tests/acceptance/vm_reclamation_claim_lifecycle.rs`
(NEW file, default lane — DELIVER's own filename call, same file-TBD
convention already used for S-VM-67), matching S-VM-77/79's existing
`@tier1 @in-memory` tags exactly. **No tier change required** — both
scenarios already target the default (non-`integration-tests`) lane, and
`overdrive-control-plane/tests/acceptance/*.rs` is confirmed default-lane
(its own entrypoint docstring: *"Acceptance tests in this crate stay in
the default unit lane"*), matching precedent files in the same directory
(`job_stop_idempotent.rs`, `action_shim_crash_observability.rs`, etc.).

**`test-scenarios.md`'s top-of-file Driving Ports table carried the same
contradiction independently** (not just DWD-04) — its `plan_reclamation`
row's exercises column read *"S-VM-21…32, S-VM-77…80 (in-memory half)"*,
grouping S-VM-77/79 under the `plan_reclamation` pure-function port they do
not drive through. Corrected in the same pass: that row now lists only
S-VM-78/80; a new row names the `overdrive-control-plane`-resident driving
ports (the exit observer's loop body / `execute_reclaim_allocation`) and
lists S-VM-77/79 against it.

### Gap 2 — S-VM-09/S-VM-19: omission, not contradiction

**Verified before fixing.** Both scenarios' own `**Driving port**:` lines
already read `overdrive deploy` (direct CLI handler call, real in-process
`overdrive serve`) — identical wording to their Tier-3 CLI-driven
neighbours (S-VM-01…05, S-VM-11…15, etc.) — and both carry `@tier3
@real-io` tags. Nothing about their placement is wrong; DWD-04's Tier-3
row's span notation (`S-VM-01…05, 11…15, 33…66, 68`) simply never named
the two IDs sitting in its own sub-range gaps (06-10 for S-VM-09, 16-32
for S-VM-19) — the same span-notation blind spot DWD-13 already found and
fixed once for S-VM-41 (there, the gap ID fell *inside* an existing
sub-span and needed no edit; here, S-VM-09/19 sit *between* named
sub-spans and the span text never mentions them at all).

**Remedy chosen: extend the existing row's span, not a new row.** The
mechanism, crate, file pattern, and rationale are byte-identical to the
row's other members — S-VM-09 and S-VM-19 are not a distinct placement
class, they were simply never named. A blanket range widen (e.g.
`01…19`) was rejected: S-VM-06/07 (parse-boundary, `overdrive-core`),
S-VM-08 (pure-function, `overdrive-core`), and S-VM-10 (schema-evolution
file edit, `overdrive-core`) all sit inside `06-10`/`16-20` and are placed
elsewhere by DWD-04's other rows — a range widen would silently
mis-claim them into the Tier-3 CLI-driven row. S-VM-09 and S-VM-19 are
named explicitly instead, mirroring DWD-13's own explicit-exclusion
precedent (S-VM-67 removed from this same row by name) applied in the
opposite direction (explicit inclusion).

**Fix**: DWD-04's Tier-3 CLI-driven row's span corrected to `S-VM-01…05,
09, 11…15, 19, 33…66, 68`. `test-scenarios.md`'s top-of-file Driving Ports
table's `overdrive deploy` row carried the identical omission (its
exercises column also silently excluded S-VM-09/19) and is corrected the
same way.

### No tier change required for either gap

Both remedies were checked against the "stop and report" instruction: a
placement fix that would require changing a scenario's tier is out of
scope for this entry. Neither did — S-VM-77/79 stay `@tier1 @in-memory`
(the target crate's `tests/acceptance/` is confirmed default-lane);
S-VM-09/19 stay `@tier3 @real-io` (the target crate/row is unchanged,
only the span notation is corrected). Nothing was stopped or reported as
blocked.

**Files touched by this entry**: `distill/wave-decisions.md` (this entry;
DWD-04's two edited rows; Changelog, below); `distill/test-scenarios.md`
(top-of-file Driving Ports table — the `overdrive deploy` row's exercises
column, the `plan_reclamation` row's exercises column, and one new row for
the `overdrive-control-plane`-resident driving ports; no Gherkin, tag,
tier, or AC changed on any scenario). `deliver/roadmap.json`, every ADR,
`brief.md`, and every `.rs` file were NOT touched — the roadmap pass is
concurrent and out of this entry's scope, and no production code exists
yet for either gap's driving ports (DELIVER scaffolds them per DWD-06).
Scenario count re-verified mechanically unchanged at **88**
(`grep -c '^#### S-VM-'` and `grep -c '^\*\*Tags\*\*:'`, both 88). No
GitHub issue created; #259–#263 remain the only real numbers in scope,
#264 closed, none newly cited here. No commit made by this pass.

---

## DWD-17: `@requires-kvm` capability-class gate — a decision the spike
explicitly deferred to this wave, closed (2026-08-11)

### The gap

`spike/findings.md` § "The nested-virt stall — SETTLED 2026-08-10 by
removing the nesting" states, verbatim: *"So: don't gate on Lima for
microVM boot. The decision point is Slice 01's first integration test,
not now. The cheap move there is a preflight that detects nested-Apple
and emits a third outcome — cannot render a verdict — rather than a
[red]."* Neither `test-scenarios.md` nor this file mentioned nested
virtualisation, KVM, or a capability gate anywhere before this entry —
confirmed by grep before acting. The decision the spike deferred to
"Slice 01's first integration test" was never picked up by the original
DISTILL pass, and survived two adversarial review rounds (DWD-08, DWD-11)
untouched, because neither reviewer was scoped to cross-check the spike's
own deferred-decision language against the scenario catalogue.

### The measured asymmetry (why this is not cosmetic)

- **Bare-metal x86_64 (env B, non-nested)**: 12/12 CH boots, 0 failed,
  time-to-init 0.730s–0.746s (16ms spread).
- **Nested aarch64 (env A, the standard macOS dev Lima VM)**: ~1 in 3
  stalls, freezing before `/init` at the root-mount boundary, killed by a
  90s watchdog.
- **CI (`ubuntu-latest`)**: real `/dev/kvm`, non-nested — unaffected.
- The spike's own framing: *"A green run is genuine evidence... A red run
  is uninformative"* — a stall and a real regression are
  indistinguishable under nested virtualisation. Per
  `.claude/rules/testing.md`, "Flaky tests break mutation testing...
  worse than missing them" applies with full force here.

### Why `@tier3`/`@real-io` alone does not capture this

The existing tags mean *"real infra that works inside Lima"* — netns,
cgroups, subprocesses, cgroupfs, real filesystem probes, `SimVmm`-free
adapter calls. That predicate does not distinguish a real
`cloud-hypervisor` guest-boot ATTEMPT (which touches `/dev/kvm`, creates
vcpus, and is exactly the capability class the spike measured as flaky
under nesting) from every other kind of real I/O this suite already
exercises safely inside Lima. A macOS developer running the standard
`--features integration-tests` command today eats ~1/3 flaky failures on
every scenario that happens to boot a guest, with no signal distinguishing
"the driver is broken" from "the dev host cannot render a verdict."

### The classification method

Walked all 88 scenarios; classified the 61 carrying `@tier3`/`@real-io`
(27 pure `@tier1`/`@in-memory` scenarios are trivially excluded — no real
I/O of any kind). Discriminator applied: **does the scenario's own
Given/When/Then necessarily require a real `cloud-hypervisor` process to
be spawned with intent to boot a guest kernel** — including scenarios
where the guest deliberately never reaches userspace (the boot ATTEMPT
itself, not completion, is what exercises KVM and is subject to the
stall)? Applied consistently:

- **YES** (`@requires-kvm` added): the allocation reaches Running, a
  guest command executes/exits/reports over vsock, a guest kernel panics
  or hangs mid-boot, confinement is inspected on a live `/proc/<vmm-pid>`,
  a storage daemon serves a live guest, in-guest CPU/memory is observed,
  or (for `vmm_equivalence.rs`, S-VM-90) `Vmm::create()` is driven against
  the real `CloudHypervisorVmm` adapter.
- **NO** (tag withheld): `overdrive serve`'s own boot-time composition
  gate/capability probe (S-VM-11, 12, 75 — checks binary presence,
  Landlock/KVM capability flags, an executed `FICLONE` ioctl; none spawn
  a guest-booting hypervisor), `SimVmm`-injected fault scenarios (S-VM-13,
  51 — the fake adapter is substituted at the port boundary, so no real
  `cloud-hypervisor` process is ever involved), pre-spawn
  artifact-validation rejections where the platform's own design states
  the check happens "before any hypervisor process is spawned" (S-VM-33,
  34, 35, 41, 58, 59 — missing/wrong-format kernel, missing rootfs, a
  vanished binary caught by `exec` failing immediately, missing volume
  source/storage-daemon), admission-time rejections stated explicitly as
  occurring "before anything is scheduled" (S-VM-38), and adapter
  equivalence tests over generic host primitives with no guest/kernel/
  hypervisor language in their Given/When/Then (S-VM-91 `VmHostState`,
  S-VM-93 `CgroupAccounting` — cgroupfs and filesystem operations that do
  not depend on what process occupies the scope), plus S-VM-94 where the
  scenario's own `Then` states explicitly *"no hypervisor process was
  spawned"* (the FICLONE clone fails before CH would ever be exec'd).

**Hybrid scenarios** (S-VM-21, 22, 25 — each carries a primary
`@tier1`/`@in-memory` shape plus a documented "Tier-3 companion" clause):
`@requires-kvm` is attached only to the companion-shape tag cluster
inline in the `**Tags**:` line, not as a scenario-wide tag — the primary
in-memory shape (`SimVmHostState` + `SimClock`) never spawns real CH.

### Genuinely ambiguous — flagged, not silently resolved

Five dispositions rest on judgment rather than an explicit textual
statement, and are recorded here rather than buried in a silent tag:

1. **S-VM-04, S-VM-39** ("A VM workload deploys through the same verb...",
   "A VM job spec is accepted") — the `Then` clause asserts only
   "accepted and scheduled," not that the guest reaches Running or
   completes. Tagged `@requires-kvm` because both sit in the
   `@happy_path` walking-skeleton-adjacent family (DWD-01: "sharing the
   same driving port and fixture shape" as S-VM-01) and this project's
   `[job]` driver dispatches to immediate execution — but the scenario
   text alone does not pin whether the crafter's test needs to observe
   guest-boot completion or only the admission/scheduling decision.
2. **S-VM-40** ("A scheduled VM job is accepted", `[schedule]`+`[vm]`,
   cron-triggered) — left WITHOUT `@requires-kvm`: a cron-scheduled job is
   deferred by construction, so "accepted and scheduled" plausibly means
   admission-only within the scenario's own timeframe, unlike S-VM-39's
   immediate `[job]`.
3. **S-VM-23, S-VM-28, S-VM-30** — each Given describes "surviving"/
   "leftover" VM artifacts "from a prior boot" as a PRECONDITION. Tagged
   `@requires-kvm` on the reasoning that a genuinely real prior boot is
   the more faithful reading (consistent with "No Fixture Theater" and
   the Tier-3-companion family's stated purpose of proving convergence
   against real infrastructure) — but a crafter could legitimately
   fixture-craft the leftover cgroup scope/directory/row state as a
   precondition without an actual prior `cloud-hypervisor` boot, since
   Given-step fixture crafting of PRECONDITIONS is sanctioned by this
   project's own testing discipline.
4. **S-VM-58, S-VM-59** (missing volume source / missing storage daemon)
   — left WITHOUT `@requires-kvm` by analogy to S-VM-33/34's
   pre-spawn-validation pattern, but neither scenario's own text states
   explicitly (the way S-VM-17/41 do for the kernel path) that the check
   precedes hypervisor spawn.
5. **S-VM-91, S-VM-93** (`VmHostState`/`CgroupAccounting` adapter
   equivalence) — left WITHOUT `@requires-kvm`: both operate on generic
   cgroupfs/filesystem primitives whose fixture could plausibly be a
   lightweight stand-in process rather than a genuine CH boot, but the
   scenario text does not rule out a real-VM-derived fixture either.

**These five items are the concurrent roadmap pass's to reconcile against
its own step criteria** — if the roadmap pass's pinned step language
implies a different disposition for any of the five, that is not a
contradiction with this entry; it is exactly the kind of judgment call
this section exists to surface rather than bury.

### What this decision does NOT do

Per the dispatch's explicit instruction, this entry does not invent the
Rust-side mechanism (a Cargo feature, a runtime preflight check, an
`#[ignore]`-with-reason convention, or a `SKIP` outcome distinct from
pass/fail). `@requires-kvm` in `test-scenarios.md` is a DISTILL-side
specification-level classification; the concrete gating mechanism that
consumes it is pinned by the concurrent `deliver/roadmap.json` pass. This
is not a deferral requiring a GitHub issue — the mechanism naming is
in-feature work assigned to the roadmap pass, not a future-phase
scope cut.

**Addendum (2026-08-12) — the mechanism is now pinned: `@requires-kvm` ⇒
the `kvm-tests` Cargo feature.** The concurrent roadmap pass named it
(first declared at `deliver/roadmap.json` step covering S-VM-90/94,
"CloudHypervisorVmm + SimVmm adapters — probe, create, terminate", on
`crates/overdrive-host`, and re-declared on `crates/overdrive-cli` at the
walking-skeleton step). See DWD-18 below for the naming rationale and the
reconciliation of this artifact's own `@requires-kvm` dispositions against
the roadmap's file-level gate.

### Files touched by this entry

`distill/test-scenarios.md` — 45 of the 61 `@tier3`/`@real-io` scenarios
gained `@requires-kvm` (a top-of-file explanatory note added; three
hybrid scenarios' companion-shape clauses tagged inline; no scenario's
tier, ACs, or Gherkin body changed; no scenario added, removed, or
renumbered). `distill/wave-decisions.md` (this entry; DWD-04's rows
annotated with the capability-lane split where it was previously
invisible; Changelog, below). `deliver/roadmap.json`, every ADR,
`brief.md`, and every `.rs` file were **not** touched — the roadmap pass
is concurrent and owns the exact mechanism/feature-name pinning; this
entry reports that the pinned name is needed for the two artifacts to
agree, per the dispatch. Scenario count re-verified mechanically
unchanged at **88** (`grep -c '^#### S-VM-'` and `grep -c
'^\*\*Tags\*\*:'`, both 88, both before and after this pass). No GitHub
issue created; #92, #222, #248, #257, #259–#263 remain the only real
numbers in scope, #264 closed, none newly cited here. No commit made by
this pass.

---

## DWD-18: `@requires-kvm` ⇒ `kvm-tests` — binding recorded, ten flagged
ambiguities reconciled against the roadmap's file-level gate (2026-08-12)

DWD-17 classified 45 of 88 scenarios `@requires-kvm` and explicitly left
the Rust-side mechanism to the concurrent `deliver/roadmap.json` pass. That
pass has now landed and pinned the mechanism. This entry (a) states the
binding, (b) records the naming reasoning so it is not re-litigated, and
(c) reconciles DWD-17's five genuinely-ambiguous dispositions (ten
scenario IDs) against the roadmap's actual file-level gate. No scenario's
tier, tags, ACs, or Gherkin body changed by this entry. No scenario was
added, removed, or renumbered. Scenario count re-verified mechanically
unchanged at **88** (`grep -c '^#### S-VM-'` and `grep -c
'^\*\*Tags\*\*:'` against `distill/test-scenarios.md`, both 88).

### The pinned name — `kvm-tests`

- **`bare-metal-tests` was rejected as false.** CI runs `ubuntu-latest` —
  a GitHub-hosted VM, not bare metal — and the spike measured it
  trustworthy ("real `/dev/kvm`, no nesting"). A "bare-metal" name would
  exclude the only host the lane actually runs on.
- **The property is nesting, not hardware.** The spike's own diagnosis
  (`spike/findings.md`): *"Nested virtualisation on Apple Silicon was the
  artifact… the Lima guest is arm64 **and** nested. The **nesting**
  caused it."* The dev Lima VM *has* `/dev/kvm` (0666 udev rule) and is
  still untrustworthy — hardware presence alone does not predict the
  stall.
- **`kvm-tests` names the lane, not a hardware claim** — shaped like its
  sibling `integration-tests` (a lane noun, not a predicate), generic
  enough to be reused by any future work needing a real guest VM
  (snapshot/restore, the kernel matrix, a future VMM) without a rename.
- **Division of labour**: the *feature* gates the lane; the *preflight*
  enforces the capability (`systemd-detect-virt` plus the `/dev/kvm`
  permission shape — the spike recorded `crw-rw---- root:kvm` 0660 as
  "the production-realistic shape" against Lima's 0666). The feature name
  does not need to encode the nesting nuance because the preflight is
  the layer that does.
- **Declaration is narrow, not workspace-wide** — only `overdrive-host`
  and `overdrive-cli` declare `kvm-tests = []`. `integration-tests` is
  universal only because `cargo-mutants` v27 scopes per-mutant builds to
  `--package`, and Tier-3 tests are structurally excluded from mutation
  runs anyway (`.claude/rules/testing.md` § "What it's NOT for") — so the
  forcing function that makes `integration-tests` universal does not
  apply to `kvm-tests`.
- **The preflight fails loudly rather than skips.** The feature gate is
  the spike's own "third outcome" (cannot render a verdict, rather than a
  red) applied at compile time; the preflight only covers the deliberate
  opt-in case, where a silent green would be the "reads green vacuously"
  trap this project's own conventions (`vm_fixture.rs`'s wiring note)
  already guard against elsewhere in this same roadmap.

### Ten-way reconciliation against the roadmap's file-level gate

The roadmap gates `@requires-kvm` at **file** granularity (every scenario
in a given test file shares that file's `kvm-tests` compile gate — or
lack of one). Only one direction is dangerous: DISTILL leaned
`@requires-kvm` but the roadmap's file placement is **not** gated — the
scenario would then compile and run on every host, including a nested
Lima dev VM, with no feature-level defense against the ~1-in-3 stall.
The opposite direction (leaned NOT, file **is** gated) is harmless
over-gating — a laptop developer skips a test that didn't strictly need
gating, nothing worse.

| Scenario | DWD-17 disposition | Roadmap file | File-level gate | Direction |
|---|---|---|---|---|
| S-VM-04 | leaned `@requires-kvm` | `vm_walking_skeleton.rs` | `integration-tests,kvm-tests` | consistent |
| S-VM-39 | leaned `@requires-kvm` | `vm_boot_failure_vocabulary.rs` | `integration-tests,kvm-tests` (inherited from step 03-01) | consistent |
| S-VM-23 | leaned `@requires-kvm` | `vm_reclamation_tier3.rs` | `integration-tests,kvm-tests` | consistent |
| S-VM-28 | leaned `@requires-kvm` | `vm_reclamation_tier3.rs` | `integration-tests,kvm-tests` | consistent |
| S-VM-30 | leaned `@requires-kvm` | `vm_reclamation_tier3.rs` | `integration-tests,kvm-tests` | consistent |
| S-VM-40 | leaned NOT | `vm_boot_failure_vocabulary.rs` | `integration-tests,kvm-tests` (inherited) | harmless (over-gated) |
| S-VM-58 | leaned NOT | `vm_volumes_and_storage_daemon.rs` | `integration-tests,kvm-tests` (inherited from step 05-01) | harmless (over-gated) |
| S-VM-59 | leaned NOT | `vm_volumes_and_storage_daemon.rs` | `integration-tests,kvm-tests` (inherited from step 05-01) | harmless (over-gated) |
| S-VM-91 | leaned NOT | `vm_host_state_equivalence.rs` | `integration-tests,kvm-tests` | harmless (over-gated) |
| S-VM-93 | leaned NOT | `cgroup_accounting_equivalence.rs` | `integration-tests` only (**deliberately not** `kvm-tests` — roadmap's own words: *"it never goes through the `Vmm` port and never spawns cloud-hypervisor, so it stays gated by `integration-tests` alone, deliberately NOT `kvm-tests`"*) | consistent |

**Result: zero scenarios land in the dangerous bucket.** No file
placement in the roadmap under-gates a scenario DWD-17 judged to need a
real guest-boot attempt. Five of the ten (S-VM-04, 39, 23, 28, 30) are
consistent — DISTILL's lean matches the roadmap's gate exactly. The
remaining five (S-VM-40, 58, 59, 91, 93) all sit in files the roadmap
placed alongside `@requires-kvm` siblings from the same AC/step (`vm_boot_
failure_vocabulary.rs` gains its gate from S-VM-39's own presence;
`vm_volumes_and_storage_daemon.rs` from S-VM-57/64/65's real-boot
scenarios; `vm_host_state_equivalence.rs` from being the file-level
adjacent-scenario neighbor of a real-VM-derived fixture) — four of those
five are over-gated (harmless), and one (S-VM-93) is correctly ungated,
matching DWD-17's own disposition exactly. Nothing in DWD-17's own
classification required correction; nothing in the roadmap's file
placement is flagged as wrong.

### Two deliberately-ungated files — confirmed to agree with DWD-17's tags

- **`cgroup_accounting_equivalence.rs` (S-VM-93)** — synthetic cgroup
  fixtures, never touches the `Vmm` port. Roadmap: gated `integration-
  tests` only, explicitly not `kvm-tests`. DWD-17: leaned NOT
  `@requires-kvm` (generic cgroupfs/filesystem primitives). **Agree.**
- **`vm_storage_daemon_sandbox_arg.rs` (S-VM-67)** — Tier-1 pure
  rendering proptest. Roadmap: "Tier 1 pure/proptest and spawns no VMM —
  it is deliberately NOT gated behind `kvm-tests`, only the crate's
  normal default lane." `test-scenarios.md`: S-VM-67 carries `@tier1
  @in-memory` (per DWD-13's rewrite to the launch-argument-construction
  layer) — not `@tier3`/`@real-io` at all, so `@requires-kvm` was never a
  candidate tag for it. **Agree.**

### Files touched by this entry

`distill/wave-decisions.md` (this entry; a short addendum inside DWD-17's
"What this decision does NOT do" section pointing here; Changelog,
below), `distill/test-scenarios.md` (top-of-file `@requires-kvm`
explanatory note — states the `kvm-tests` binding explicitly, replacing
the now-stale "pinned by the concurrent roadmap pass" forward pointer; no
scenario body, tag, tier, or AC touched). `deliver/roadmap.json` was
**read only** — not touched, per this dispatch's explicit instruction; it
is the artifact that pinned the name this entry reconciles against. No
ADR, `brief.md`, or Rust file touched. Scenario count re-verified
mechanically unchanged at **88** (`grep -c '^#### S-VM-'` and `grep -c
'^\*\*Tags\*\*:'`, both 88, both before and after this entry). No GitHub
issue created; #92, #222, #248, #257, #259–#263 remain the only real
numbers in scope, #264 closed, none newly cited here. No commit made by
this pass.

---

## DWD-19: Mutation-testing cadence — per-step step-AC mutation gates superseded by one end-of-DELIVER whole-diff gate (2026-08-14, user-approved)

Several DELIVER step ACs pin a **per-step** `cargo xtask mutants` run as a
step-close gate. The worked example is roadmap step 01-05 AC #4:
*"`cargo xtask mutants --file crates/overdrive-core/src/vm/config.rs`, scoped
to `reserve_bytes` AND the `MemoryPlan::cgroup_max_bytes` invariant, is run and
its kill-rate gate passes before this step closes."* By user approval
(2026-08-14), those per-step runs are **superseded** by a single
**end-of-DELIVER, per-feature mutation gate** over the whole phase diff —
`cargo xtask mutants --diff origin/main`, kill-rate ≥ 80%, run once before the
feature's PR opens.

**Scope — general, every affected step, not only 01-05.** This applies to
*every* DELIVER step in this feature whose acceptance criteria named a
per-step `cargo xtask mutants` run. The cadence for all of them collapses to
the one end-of-DELIVER whole-phase-diff gate; no step is closed on — or blocked
by — its own individually-scoped mutation run.

**Why the coverage is equivalent.** `cargo xtask mutants --diff origin/main`
mutates every mutable operator site in the feature's whole branch diff against
`main`. `reserve_bytes`, `MemoryPlan::derive`, and the
`MemoryPlan::cgroup_max_bytes` invariant land in the Slice-01 (step 01-05)
portion of that diff, so the end-of-DELIVER whole-diff run mutates them exactly
as a per-step `--file crates/overdrive-core/src/vm/config.rs` run would have.
Only the *timing/cadence* differs — once, at end-of-DELIVER, rather than once
per step — never the coverage. (`.claude/rules/testing.md` § "Per-step vs
per-PR scoping" names the cost the per-step cadence pays: it re-mutates earlier
commits' code repeatedly across a multi-step feature; the single end-of-phase
run is that section's own "final check before opening the PR.")

**Authority.** CLAUDE.md § "Mutation Testing Strategy" — *"This project uses
per-feature mutation testing. Per-PR runs are diff-scoped via `cargo mutants
--in-diff origin/main` with a kill-rate gate of ≥80%."* — plus the user
directive of 2026-08-14. The end-of-DELIVER whole-diff gate **is** that project
policy; the per-step step-AC gates were the local anomaly this decision
reconciles back to it.

**Unchanged.** The `@mandatory:mutation_target` *tags* on scenarios (S-VM-18,
S-VM-20, and every other scenario carrying one) still attach **per-step** — the
tag marks a symbol as a mutation target at the step that lands its body, and
ADR-0082 §D2.3's note that the mutation obligation "attaches at the DELIVER
step that measures it" still governs *which* step a tag attaches to. This entry
governs only *when the run that discharges those tags happens*: it is deferred
from per-step to the single end-of-DELIVER whole-diff gate. A cadence decision,
not a coverage decision.

**Files touched by this entry.** `distill/wave-decisions.md` (this entry +
Changelog, below). `deliver/roadmap.json` — a single one-line pointer appended
to step 01-05's `implementation_notes` noting the supersession and pointing
here; the step `criteria` arrays are **not** rewritten (this wave-decision is
the authoritative reconciliation, and the criteria text is left as the
historical AC — avoids churning the DES artifact). No ADR, `test-scenarios.md`,
`brief.md`, or Rust file touched by this entry. No scenario added, removed,
renumbered, or re-tagged. No GitHub issue created.

---

## DWD-20: 01-07 review reconciliations — the `Driver` post-stop `status()` contract (item 1), the unwired `EXEC` write (item 2), two 01-06 accessors blessed (item 3) (2026-08-14)

Three design items surfaced by the step 01-07 review, ruled and pinned in
the design SSOT. All three are reconciliations of already-shipped 01-07/01-06
code against the design; none required a user scope decision. **No Rust file
touched by this entry** — the crafter-facing instructions are handed to the
01-07 review-remediation dispatch; the pins below are the authority the crafter
implements to.

### Item 1 (PRIMARY) — the `Driver` post-stop `status()` contract vs § 105a.3's exit-watcher-owned claim

**The conflict.** The base `Driver` trait binds every implementor
(`crates/overdrive-core/src/traits/driver.rs`, post-stop contract): *after
`stop()` returns `Ok(())`, a subsequent `status()` returns `Err(NotFound)`.*
The shipped `VmDriver::stop` (`crates/overdrive-worker/src/vm_driver.rs`,
commit `e4f6602e`) extracts the live state but leaves the live-map entry
`Live`, so `status()` returns `Running` until the exit watcher happens to fire
— a real violation. Reading the conflict as "§ 105a.3 gives the watcher
*exclusive* ownership of `Live → EndingInFlight`, so `stop` cannot drive it"
would make the trait contract unsatisfiable for `VmDriver`.

**Ruling — a refinement that dissolves the conflict (the reviewer's option
(c), which is the "marks the claim" half of option (a), and the *only* correct
reconciliation).** `VmDriver::stop` transitions the entry `Held → EndingInFlight`
(new transition **3b**, brief § 105a.3) under the same lock it extracts the
live state with. Because `VmDriver::status` already maps `EndingInFlight →
NotFound`, the trait contract holds **synchronously**; because `live_allocations()`
reports `EndingInFlight`, the authorship claim is **retained**, so
`VmReclamation`'s `EndingInFlightIsNeverReclaimed` (§ 105a.11) holds across the
stop→terminal-row window. This is not an invention: § 105a.11's own invariant
wording already names *"or its stop has been issued"* as an `EndingInFlight`
trigger — the FSM already presupposes this transition; the shipped code merely
never performed it.

**Why the three offered options resolve to exactly this one:**
- **NOT option (a)'s full removal (the `ExecDriver` mirror).** `ExecDriver::stop`
  may remove its entry because nothing consults its supervision set
  (`live_allocations() → None`). `VmDriver`'s set IS consumed by `VmReclamation`,
  so removing the entry at `stop` drops the claim during the stop→terminal-row
  window and lets `plan_reclamation` author a competing `PlatformReclaimed`
  ending — the NEW-1 failure, a direct violation of `EndingInFlightIsNeverReclaimed`.
  This is the load-bearing reason `VmDriver` must differ from `ExecDriver`.
- **NOT option (b)'s separate "stopping" marker.** A second field/map read by
  `status()` is exactly the "two representations of one fact that can disagree"
  shape § 105a.3 already rejected for the claim; `EndingInFlight` is the one
  representation and it already carries both meanings (status → NotFound AND
  still-claimed).
- **No `Driver`-trait carve-out is minted, and no `driver.rs` trait-docstring
  edit is needed.** `EndingInFlight` is an in-flight authorship claim released
  once the ending is authored (transitions 5/6), not the permanent
  "terminal-state memory" the contract forbids. The contract stands unweakened.

**Stop/watcher race safety.** Transitions 3 (watcher) and 3b (stop) are both
`Held → EndingInFlight` check-and-acts under the one `parking_lot` map lock.
Whoever reaches `Held` first wins: the entry becomes `EndingInFlight` exactly
once, an `ExitEvent` is emitted at most once (only if the *watcher* won — a
natural exit that beat the stop), and the loser observes non-`Held` and takes
its idempotent no-op / `NotFound` path. On the stop-wins ordering the watcher
later fails its transition and emits nothing; the operator-stop ending is
authored on the **stop path** (transition 6, the shim's terminal-row write),
which also retires the `intentional_stop: false` the watcher hard-codes (the
shim knows the stop was operator-initiated; the watcher cannot).

**Scope.** Lands as the **01-07 review-remediation** (`VmDriver::stop`/`status`
only). The release side — transitions 5/6, the exit observer's
release-on-every-arm, the shim's terminal-row authorship — is unchanged and
remains step **02-02**'s; 02-02's transition-table proptest (S-VM-77) picks up
row 3b. No existing 01-07 acceptance test breaks (verified against all eight in
`vm_driver_stop_totality.rs`); `stop_sequence_d`'s doc comment becomes
deterministic-`NotFound` and is updated by the crafter.

### Item 2 — the § D7 `EXEC` write is not yet wired; ownership reaffirmed as 01-07 (scoped addendum, NOT reassigned to 01-08)

ADR-0082 § D7's ownership table already assigns *"VmDriver **writes** `EXEC` on
the § D3 Ok continuation"* to **step 01-07** — but 01-07's four roadmap criteria
never encoded it, and the shipped `VmDriver::start` beacon-win arm stores the
session + spawns the watcher WITHOUT writing `EXEC`, so a 01-08 boot would leave
the guest (which already blocks on one `EXEC`, landed 01-03) waiting forever.

**Ruling: scoped addendum to 01-07 (the reviewer's second option), not
reassignment to 01-08.** `vm_driver.rs` is 01-07's scope, not 01-08's, and the
§ D7 table already owns the write there — reassigning would *contradict* the
table. Roadmap 01-07 gains a fifth criterion for the `EXEC` write (mechanism:
write `EXEC <serde_json argv>` from `spec.command`/`spec.args` via the landed
`BeaconMessage::Exec` `Display`; write-err → `StartRejected` with full cleanup +
claim release; `LiveVm.beacon` stored `Some` only after the write succeeds). The
operator command **source** (`[vm]+[job]` → `AllocationSpec.command`/`args` via
`DriverInput::Vm`) and the real-guest **proof** stay **01-08** (S-VM-01), exactly
as the § D7 table's last two rows assign; `EXEC`'s first real evidence remains
the 01-08 Tier-3 walking skeleton. ADR § D7 gained an amendment stating the
mechanism is not-yet-wired and naming 01-07 as the wiring step; the "folded into
§ D3's Ok continuation and gates Running" design text is unchanged (it is
correct, just not yet implemented).

### Item 3 (trivial) — two 01-06 first-implementor accessors blessed as sanctioned surface

ADR-0082 § D2.4 gained a one-line "gap 6" amendment (matching gaps 1–5 of the
2026-08-12 amendment) blessing `KernelImage::path(&self) -> &Path`
(`config.rs:201`, sibling of `RootfsPlan::master()` / `KernelCmdline::as_str()`)
and `VmExitWatch::new(oneshot::Receiver<VmmExit>) -> Self` (`vmm.rs:202`, the
structurally-forced sole constructor for the § D3 private-field return type).
Both accessors verified present in tree; the 01-06 review already ACCEPTED both
on substance. Documentation-trail consistency only.

**Files touched by this entry.** `distill/wave-decisions.md` (this entry +
Changelog). `docs/product/architecture/adr-0082-…md` (§ D2.4 gap-6 note, § D4
item-1 amendment, § D7 item-2 amendment). `docs/product/architecture/brief.md`
(§ 105a.3 transition 3b + item-1 amendment blockquote). `deliver/roadmap.json`
(step 01-07: two review-remediation criteria; no `implementation_scope` change
needed — `vm_driver.rs` and `traits/driver.rs` are already in 01-07's scope). No
`test-scenarios.md` or Rust file touched. No scenario added, removed,
renumbered, or re-tagged. No GitHub issue created.

---

### DWD-21 — Guest vsock provisioning: overdrive-init loads the modules on the stock-kernel test path; the appliance kernel builds vsock in (2026-08-14, DESIGN ruling — Morgan, GH #42)

**Recorded into the DISTILL log by the DESIGN wave (`nw-solution-architect`)
per an explicit dispatch**, because step 01-08's walking skeleton surfaced a
cross-step design decision the crafter had no authority to invent
(`vm_walking_skeleton.rs` module doc; CLAUDE.md § "Implement to the design").
Docs-only; the code lands via the 01-08 review-remediation dispatch.

**The blocker.** A real `[vm]+[job]` boot reaches the guest, but
`overdrive-init`'s `socket(AF_VSOCK, …)` fails `EAFNOSUPPORT`, so the guest
never dials the beacon and the allocation never reaches Running. Real console,
metal box, kernel 7.0.0-15-generic, 2026-08-14:

```
[    0.781590] Run /sbin/init as init process
overdrive-init: fatal: could not create the beacon vsock socket: EAFNOSUPPORT: Address family not supported by protocol
[    0.786611] reboot: Power down
```

`#[ignore]`s S-VM-01/02/05/74 in `crates/overdrive-cli/tests/integration/vm_walking_skeleton.rs`.

**Investigation (from the artifacts, not assumed).**

1. **The spike SOLVED this via module-load — it is a spike→ship regression,
   not a new capability.** `spike-scratch/increment-a/build.sh:76-84` stages
   three vsock `.ko` (`vsock`, `vmw_vsock_virtio_transport_common`,
   `vmw_vsock_virtio_transport`), zstd-decompressed from
   `/lib/modules/$(uname -r)/kernel/net/vmw_vsock/*.ko.zst`;
   `probe/src/bin/guest_init.rs:82-106,219-231` `finit_module(2)`-loads them
   in dependency order **before** dialing. The spike's kernel is the box's
   stock distro `vmlinuz` (vsock=m), NOT a vsock=y build. `findings.md` § P2
   proves the beacon works this way (`/dev/vsock present BEFORE insmod =
   false → insmod ×3 OK → present AFTER = true`; READY + EXIT 7 received in
   order), and the **12/12 bare-metal run is a genuine vsock end-to-end
   proof** — not something else. `[D2]` (findings § "Design implications"):
   *"Either set them built-in in the appliance kernel, or `overdrive-init`
   must `finit_module` three `.ko`s in dependency order before the beacon.
   Built-in is strongly preferable."*
2. **The 01-04 fixture lost BOTH halves.** `vm_fixture.rs::stage_kernel`
   (`:542-572`) stages the stock `/boot/vmlinuz-$(uname -r)` (vsock=m)
   verbatim; `build_staging_tree` (`:716-745`) stages `/sbin/init`, `/init`,
   `/dev/console`, `/dev/null` — and **no `.ko` modules**. The module doc
   (`:97-108`) explicitly flags this as a "Known gap this step does NOT
   close," deferred to "whichever step first drives a REAL guest boot."
3. **`overdrive-init` (01-03) loads nothing.** `main.rs::connect_beacon`
   (`:159-170`) goes straight to `socket(AF_VSOCK)` → `connect`; the crate is
   `#![forbid(unsafe_code)]` and has no `finit_module`.
4. **Production model (ADR-0068).** Overdrive "controls the kernel image it
   boots" — a pinned, controlled 6.18-LTS appliance build. So **vsock=y
   built-in is the natural production answer**. But ADR-0068 was silent on
   vsock, and the spike's P3 (pinned-kernel boot) was **never run** — every
   measured boot used the stock vsock=m kernel.

**Ruling — (c): reconcile both, each on the kernel it applies to.** The
spike's (b) module-load and production's (a) vsock=y are NOT in conflict —
they target two different kernels:

- **Stock-kernel TEST path (the fixture) → module-load, exactly as the spike
  proved.** `overdrive-init` `finit_module`-loads the three staged `.ko` in
  dependency order before `connect_beacon`, tolerating already-loaded
  (`EEXIST`) and absent; the 01-04 fixture stages those three `.ko`
  (zstd-decompressed) from the **same `uname -r`** the staged kernel came
  from (no rootfs↔kernel skew). The fixture cannot practically make the stock
  distro kernel vsock=y without building a custom kernel — out of its stated
  scope, and a far larger lift than the proven mechanism.
- **Production APPLIANCE kernel (ADR-0068) → vsock built-in.**
  `CONFIG_VSOCKETS=y` + `CONFIG_VIRTIO_VSOCKETS=y` (+ host-side
  `CONFIG_VHOST_VSOCK=y`), per `[D2]` "built-in is strongly preferable."
  Recorded as ADR-0068 §4 (amendment 2026-08-14).

The two coexist in **one** `overdrive-init` binary with no `#[cfg(test)]`
branch: a vsock=y image simply stages no modules, so the load is a no-op and
`connect_beacon` proceeds directly. This is **not** "production shaped by
simulation" (development.md): it is the documented `[D2]` fallback and genuine
kernel-config-variance resilience; the audit checklist passes (an absent
module dir is one `read_dir`→`ENOENT`, no production degradation,
self-explaining from the ADR-0082 §D4 contract). It is also fail-closed /
Earned-Trust honest: a load failure is a typed `InitError` on `/dev/console`,
and a still-unavailable `AF_VSOCK` is the existing `EAFNOSUPPORT` typed error,
never a silent hang.

**Ownership — 01-08 review-remediation.** The walking skeleton is 01-08's to
make GREEN (CLAUDE.md § "Build vertical slices"), so the fix rides 01-08 even
though it touches two cross-step files, now added to step 01-08's
`implementation_scope.source_directories`:

- `crates/overdrive-init/src/main.rs` — module-load before `connect_beacon`
  (`nix::kmod::finit_module`, nix "kmod" feature, zero-unsafe preserved).
- `crates/overdrive-testing/src/vm_fixture.rs` — `build_staging_tree` stages
  the three `.ko` into the shared pinned in-guest dir.

ACs the fix satisfies: **unblock** S-VM-01/02/05/74 (remove `#[ignore]`; the
guest reaches READY and the exit code reaches the operator); **author** S-VM-15
(needs a live beacon) and S-VM-14 (deadline arm) — both already belong to
01-08 (criteria 6/7 + `scenario_name`), authored only after the fix so
S-VM-14's "never beacons" is a genuine no-beacon, not an `EAFNOSUPPORT`
false-green. Verification: a REAL `cargo xtask metal run --` boot reaching the
READY beacon (never `--no-run`).

**ADR reconciliation.** ADR-0068 gains §4 (amendment): the appliance kernel
builds vsock in, verified by the existing pinned-kernel Tier-3 leg (DWD-10),
not assumed. ADR-0082 §D4 gains an amendment (2026-08-14) pinning the loader
(`overdrive-init`), the staging-path/`uname -r` invariant, and the
no-op-on-vsock=y contract — reconciling the section's own "built-in preferable,
*or* modules load" note that never said who loads them. **ADR-0083 is
unaffected** (`Vmm`/`DriverRegistry`/allocation payload untouched). `brief.md`
§105a is **consistent** with this ruling (it already anticipates the module
load) and needs no edit; the crafter marks any adjacent stale comment the fix
makes false, naming the later step (behavior-change-marks-stale-docs rule).

**Files touched by this entry.** `distill/wave-decisions.md` (this entry +
Changelog). `docs/product/architecture/adr-0068-…md` (§4 amendment).
`docs/product/architecture/adr-0082-…md` (§D4 amendment 2026-08-14).
`deliver/roadmap.json` (step 01-08: one additive criterion, two
`source_directories`, an `implementation_notes` remediation note — additive,
no removals; JSON validity preserved). No `test-scenarios.md`, `brief.md`, or
Rust file touched by this docs pass. No scenario added, removed, renumbered, or
re-tagged. No GitHub issue created (the appliance-kernel vsock=y verification
is existing appliance-kernel Tier-3 work owned by ADR-0068 §1's tested-image
discipline / DWD-10, not a new deferral).

---

### DWD-22 — Alloc→driver index MISS broadcasts to all composed drivers; the `ShimError::UnknownDriverForAlloc` pin is retired (2026-08-14, DESIGN ruling — Morgan, GH #42)

**Recorded into the DISTILL log by the DESIGN wave (`nw-solution-architect`) per
an explicit dispatch**, closing the 01-08 review's MAJOR finding **D1** — a
design-conformance defect (the shipped shim silently contradicts a pinned ADR
contract), not a correctness landmine. Docs-only; **no code change** (the shipped
code already matches the amended contract).

**The finding.** ADR-0083 § D2a(b) pinned: *"A miss resolves to a typed
`ShimError::UnknownDriverForAlloc { alloc_id }` and is never silently routed to
`ExecDriver`."* The shipped `resolve_drivers_for_alloc`
(`action_shim/mod.rs:668-678`) does the **opposite**: on an `alloc_drivers` index
MISS it **broadcasts** the stop/terminal call to *every* composed driver. The
variant `UnknownDriverForAlloc` **does not exist anywhere in the tree** (`grep`:
named only in the ADR + `brief.md`). No prior ADR amendment / DWD / execution-log
ESCALATION documented the deviation — the 01-08 GREEN log records "AllocDriverIndex
routing" but never flags the broadcast-vs-typed-error divergence.

**Investigation (from the artifacts, not assumed).**

1. **The four consumers, classified.** `resolve_drivers_for_alloc` is read by four
   arms: `FinalizeFailed` → `on_alloc_stable`/`on_alloc_terminal` (`:1291`);
   `RestartAllocation` stop-half → `stop` (`:1604`); `StopAllocation` → `stop`
   (`:1858`) and → `on_alloc_terminal` (`:1922`). All are ending/lifecycle-authoring
   arms consumed *after* dispatch; none carries a spec, so none can re-derive the
   driver from the action alone (the whole reason the index exists).
2. **Why the miss happens — a test seam, and a per-boot production state.** The
   regression `stable_does_not_stop_probe_supervision.rs` calls
   `driver.on_alloc_running(&spec)` **directly** (`:294/:352/:395/:431`) to register
   the ProbeRunner supervisor, then dispatches `FinalizeFailed`/`StopAllocation`
   through the real `dispatch`. Because the supervisor was registered *without* a
   `StartAllocation` dispatch, the index is empty → the terminal arm MISSES. In
   production `on_alloc_running` fires **only** inside the Start/Restart arms
   (`:1572`/`:1819`), *after* the index is written — so the "supervisor live, index
   empty" state is a pure **test-seam artifact** for the lifecycle hooks. **But** the
   index is in-memory and per-boot (§ D2a(b) "Rejected"), so an empty-index miss is
   *also* a legitimate production state for any alloc not started in the current
   boot epoch (operator `stop` of a workload `Running` since before a `serve`
   restart). The crafter chose broadcast over a silent no-op or the typed error to
   keep the regression green; the review confirmed the runtime is SAFE (every
   `Driver::stop`/`on_alloc_*` is NotFound-tolerant / no-op for an alloc it does not
   own).
3. **The ADR's typed-error rationale attacked a strawman.** Its stated fear was a
   fallback that *"silently routes to `ExecDriver`"* → `ExecDriver::stop` no-ops on
   a VM alloc → GiB-scale unstoppable orphan. The shipped fallback routes to **all**
   drivers, **including the `VmDriver`** — so a VM alloc's `stop` reaches its owning
   driver and the orphan is never stranded. Broadcast satisfies the ADR's safety
   *intent* by a different mechanism than the ADR's literal *text*.

**Ruling — (a) bless the broadcast-fallback; retire the typed-error pin.** Three
independent legs:

- **The typed error's own justification does not hold against the shipped design.**
  Broadcast ≠ "route to `ExecDriver`"; it reaches the owning driver, so the specific
  orphan-stranding failure the variant was chosen to prevent *cannot occur* under
  broadcast.
- **Taken literally, the typed error inverts its safety goal.** On a legitimate
  per-boot miss (post-`serve`-restart operator stop; the probe-lifecycle test seam),
  returning `Err(UnknownDriverForAlloc)` routes the stop to **nobody** → creates the
  very orphan SD-1 prevents. A typed error is correct only if a miss is *always* a
  bug; it is not, and the shim cannot distinguish same-boot from cross-boot misses
  without a `boot_epoch` flag — the self-declared boolean § D7 item 1 rejects on the
  kill-authorising path.
- **The runtime is confirmed safe and the code already reads as broadcast.** All
  four call-site comments + the `resolve_drivers_for_alloc` docstring already
  describe the broadcast fallback and cite § D2a(b); only the ADR/brief *text*
  contradicted the tree.

**Options (b) typed error and (c) split rejected.** Both reintroduce the orphan on
exactly the stop arms (`:1604`/`:1858`) the ADR cares most about: those must reach
the owning driver on a post-restart miss, and a typed error there refuses to route.
The lifecycle arms (`:1291`/`:1922`) miss only via the test seam or post-restart
(where the ProbeRunner supervisor is *also* empty per-boot, making the hook a
harmless no-op). No arm has a miss that is "always a bug," so no arm benefits from
the typed error; (c) collapses into (b)'s failure.

**Residual, recorded (not glossed).** Broadcast masks a genuine *same-boot* miss (an
index entry lost to a real defect) that a hard error would surface, and fans the
call out to N drivers (N=2 today, Exec+Vm — negligible). The mask is unavoidable
without the rejected epoch flag; the sanctioned recovery is observability, not
refusal — an **optional, non-blocking** `tracing::debug!(name:
"shim.alloc_driver.index_miss", %alloc_id)` on the fallback arm. The broadcast's
safety leans entirely on the NotFound-tolerant/no-op `Driver` contract, which is now
declared load-bearing: if a future driver's lifecycle hook ever acquires a
non-idempotent side effect for an alloc it does not own, this ruling must be
revisited (index repopulation or a per-boot-scoped typed error).

**Crafter instruction — NO CODE CHANGE.** The shipped `resolve_drivers_for_alloc`
and all four call-site comments already match the amended contract; every comment
already cites § D2a(b) and describes the broadcast fallback. The crafter's only
action is to **confirm** those citations now resolve to the amended § D2a(b) (they
do, textually — no edit). The `ShimError::UnknownDriverForAlloc` variant is retired
and MUST NOT be implemented. The optional `index_miss` observability event above may
be added at the crafter's discretion but is not required and is not a deferral.

**ADR reconciliation.** ADR-0083 § D2a(b) amended (the typed-error paragraph
rewritten to pin broadcast + retire the variant). `brief.md` § 104's miss-disposition
sentence amended to match. § D3 (the `DriverPayload` shape) names no typed error and
is untouched; § D2a(c) (mTLS gating) untouched. No other ADR affected.

**Files touched by this entry.** `distill/wave-decisions.md` (this entry +
Changelog). `docs/product/architecture/adr-0083-…md` (§ D2a(b) amendment).
`docs/product/architecture/brief.md` (§ 104 miss-disposition sentence). No
`deliver/roadmap.json` edit — the ruling requires no code change, so no AC / source
file / step scope changes (no roadmap step describes the typed-error variant). No
`test-scenarios.md` or Rust file touched. No GitHub issue created (a design-conformance
reconciliation of an already-shipped-and-safe behavior is not a deferral).

---

### DWD-23 — `[vm]` "at admission" capability rejection: dispatch-time fallback ratified as a SAFE INTERIM; "at admission" stays required DESIGN intent, scoped to a follow-up (2026-08-14, DESIGN ruling — Morgan, GH #42)

**Recorded into the DISTILL log by the DESIGN wave (`nw-solution-architect`) per an
explicit dispatch**, closing the 01-09 review's MEDIUM finding **D2**. Docs-only;
**no code change in this entry** — the follow-up implementation is *surfaced for
user approval*, not performed here.

**The finding.** Roadmap step 01-09 AC #1: *"CH present+healthy composes the Vm
driver entry; CH absent boots the node with no Vm entry **and rejects `[vm]` at
admission naming the capability**."* Shipped behavior (verified by the review and
by S-VM-12): the CH-absent case does **not** reject at admission.
`handlers.rs::submit_workload` carries ZERO driver-capability logic — a `[vm]`
spec is admitted (`IdempotencyOutcome::Inserted`) and the capability enforcement
happens LATER at dispatch (`action_shim/mod.rs:1399-1407`: `drivers.get(kind) →
None → DriverError::StartRejected`), so the alloc reaches `Failed` naming the
capability ("no vm driver composed on this node"). **SAFE** (never silently
accepted-and-hung), but not the literal "at admission." The crafter disclosed the
gap in S-VM-12's docstring rather than inventing the gate (CLAUDE.md § "Implement
to the design").

**Investigation (from the artifacts, not assumed).**

1. **"At admission" is deliberate, cross-artifact DESIGN intent, not loose
   phrasing.** SD-5 (`brief.md` § 625, Titan): *"serve boots normally; `[vm]`
   deploys are rejected at admission naming the absent capability. Not a fault."*
   ADR-0083 §§ D1.3 / D2 / D4 (Morgan): § D4 is an entire subsection — *"The
   capability rejection is separate from the parse rejection … fails at admission,
   naming the absent capability and the node … the deploy still fails; the message
   improves."* `brief.md` § 104 repeats it. Even `DriverRegistry::kinds()`'s own
   docstring reads *"iterated for the admission-rejection message."* The surfaces
   were built FOR this gate.
2. **The safety intent IS met by the dispatch-time fallback.** S-VM-12 proves: CH
   absent → node boots, no `Vm` entry; `[vm]` deploy → alloc reaches `Failed`,
   `reason.human_readable()` contains "no"/"vm"/"composed"; never a hang, never a
   parse error. The `action_shim` arm's own comment calls this "the dispatch-time
   fallback for whatever reaches here regardless."
3. **The finding's cost premise is STALE — the gate is cheap.** The S-VM-12 note
   (and the review) state a true admission gate "would require … `AppState`
   widening to carry the `DriverRegistry`." **That widening already landed in step
   01-08**: `AppState.drivers: Arc<DriverRegistry>` (`lib.rs:200`),
   `submit_workload` already takes `State(state)`, and `DriverRegistry::{supports,
   kinds}` already exist. The only genuinely-missing pieces are a
   `WorkloadDriver::driver_type()` helper, the check in `submit_workload`, and one
   typed `ControlPlaneError` variant — a small, well-supported addition, NOT the
   AppState surgery the note implies.
4. **Step 01-09 never owned the gate.** 01-09's `implementation_scope.
   source_directories` is `lib.rs` + `cgroup_accounting.rs` + `exit_observer.rs`
   — **not `handlers.rs`**. The admission gate lives in
   `handlers.rs::submit_workload`, which 01-09 does not touch. AC #1's "at
   admission" clause was therefore **mis-scoped against its own step** —
   unbuildable within 01-09's declared surface. 01-09's correct deliverable for
   its scope IS the composition gate + dispatch-time fallback.
5. **Multi-node nuance (recorded, shapes the ruling).** The dispatch-time fallback
   is **node-correct and multi-node-ready** — at dispatch the alloc's node is
   known, so `state.drivers` is that node's capability set. A submit-time
   admission gate checks the LOCAL registry, correct for **Phase 1 single-node**
   (submit handler and the one node co-located) but in multi-node evolving into a
   **scheduler-admission** check. So the two gates are complementary, not
   redundant: the dispatch-time fallback STAYS as defense-in-depth; the admission
   gate is the Phase-1 operator fast-fail layered above it.

**Ruling — (b): "at admission" stays REQUIRED DESIGN intent; the dispatch-time
behavior is ratified as a SAFE INTERIM (not the final design); the admission gate
is scoped to a follow-up step.** Justification:

- **Do not amend deliberate, cheap-to-honor design out of the record.** "At
  admission" is specified across SD-5, ADR-0083 §§ D1/D2/D4, `brief.md` § 104, the
  roadmap AC, and even the `kinds()` docstring, with an explicit rationale (§ D4).
  Ratifying dispatch-time as *final* (option (a)) would require rewriting all of
  it AND contradicts § D4's own words ("the deploy still fails") — under
  dispatch-time the deploy *succeeds* (`Inserted`). With the gate cheap
  (finding #3), there is no cost justification for discarding the intent.
- **But the interim is genuinely SAFE (finding #2), so nothing is urgent.** This
  is a MEDIUM operator-UX + design-consistency gap, not a safety hole.
- **Option (a) rejected** (ratify dispatch-time, amend the design): discards
  deliberate intent for no cost saving, contradicts § D4's "the deploy still
  fails," and leaves a committed dead `[vm]` intent on an uncomposed node
  (perpetual `Failed` / eventual `BackoffExhausted`) the operator must manually
  stop.
- **Parser-placement variants rejected**: § D4 already ruled the capability
  rejection is NOT a parse concern (a host property must not masquerade as a spec
  property). The `[vm]`+`[service]` PARSE rejection (S-VM-38, US-VM-6 / AC-10) is
  a DIFFERENT rejection and is untouched by this ruling.

**The one genuine scope decision — SURFACED for the user, NOT decided here.**
Whether to **build the Phase-1 admission gate now** vs **defer it** (to the
multi-node scheduler-admission work) is a real sequencing call, and the follow-up
needs a GitHub issue (which requires user approval per CLAUDE.md § "Deferrals
require GitHub issues"). Tradeoff — *build now*: honor the design, better
operator-UX (synchronous "no", no dead intent), cheap; *defer*: accept the safe
interim, avoid a Phase-1-specific gate that evolves when the scheduler lands.
**Recommendation: build it as a small follow-up step** (cheap, honors deliberate
design), pending the user's go-ahead + issue approval.

**Crafter instruction (for the follow-up step, once approved).** In
`handlers.rs::submit_workload`, after the `intent` is built and BEFORE the
`state.store.put_if_absent(...)` at `:443`: derive the workload's `DriverType`
from the intent's `WorkloadDriver` (add `WorkloadDriverV2::driver_type(&self) ->
DriverType`, `Exec→Exec` / `Vm→Vm`, mirroring `DriverPayload::driver_type()` at
`traits/driver.rs:311`); if `!state.drivers.supports(dt)`, return a NEW typed
`ControlPlaneError::DriverCapabilityUnavailable { requested: DriverType, node:
NodeId, supported: Vec<DriverType> }` (message iterates `state.drivers.kinds()`
and names the node) — mapped in `to_response` to **HTTP 422 Unprocessable Entity**
with a distinct `error: "capability_unavailable"` discriminator (NOT 400
"validation" — that would blur § D4's parse-vs-capability separation; NOT a
transient 503). This is pre-`Inserted`, so nothing is committed. **Do NOT remove
the `action_shim` dispatch-time fallback** — it stays as defense-in-depth. The
follow-up step's `implementation_scope` MUST add `handlers.rs` (+ `error.rs`,
`aggregate/mod.rs`), and MUST update S-VM-12 (its assertion FLIPS: the deploy now
returns the typed rejection at admission, not `Inserted`-then-`Failed`).

**Doc reconciliation done in THIS entry (docs only).** (1) `deliver/roadmap.json`
step 01-09 AC #1 reworded to the SHIPPED dispatch-time behavior (removes the false
"at admission" claim from a step whose scope excludes `handlers.rs`) + a scope
note appended to its `implementation_notes`. (2) `brief.md` § 104 + ADR-0083
status header gain an implementation-status note: "at admission" stays design
intent; current impl is the safe dispatch-time interim; the gate is a
pending-approval follow-up. (3) Code forward-references that name a non-existent
"step 01-09 admission gate" are **SPECIFIED for correction, not applied**
(docs-only): `action_shim/mod.rs:1396-1397`, `error.rs:706` (`VmmBoot` docstring
"rejected at admission"), and the `vm_walking_skeleton.rs` S-VM-12 note — each
must point to this DWD / the follow-up step rather than "step 01-09."

**Files touched by this entry.** `distill/wave-decisions.md` (this entry +
Changelog); `deliver/roadmap.json` (step 01-09 AC #1 + `implementation_notes`);
`docs/product/architecture/brief.md` (§ 104 status note);
`docs/product/architecture/adr-0083-…md` (status-header amendment note). No Rust
file touched — the code-comment corrections and the gate itself are crafter-facing
and pending approval. **No GitHub issue created — the follow-up is SURFACED for
user build-vs-defer approval; on approval the orchestrator creates the issue and
adds the roadmap step.**

---

### DWD-24 — `StartRejected` becomes a typed failure envelope; Phase 03 resumes through an upstream-contract vertical slice (2026-08-16, DESIGN ruling — Morgan)

**Recorded into the DISTILL log by the DESIGN wave (`nw-solution-architect`)
per explicit dispatch. No code, test, execution-log, or history file is edited
by this ruling.**

**The blocker, grounded.** Checkpoint `3222f030` stopped old roadmap step
03-01 before RED and recorded that its two-file scope could not satisfy its own
S-VM-33/34/35/36/41 criteria:

1. `DriverError::StartRejected { driver, reason: String }` discarded the
   structured cause before the action shim saw it.
2. `VmDriver::start` discarded the resolved `VmmExit`, including exit code,
   signal, and stderr tail.
3. The rootfs pre-check exposed a bare I/O error with no exact configured path.
4. The boot-deadline arm had no live console-tail value.
5. Kernel validation at `serve` composition made S-VM-33 and S-VM-41's former
   initial conditions unable to create an allocation-level `Failed` row.

No production or test code landed in old 03-01. Treating that checkpoint as a
completed delivery step would therefore be false; reopening its original scope
would still be unbuildable.

**Ruling — typed contract, exact public shape.** ADR-0032 §4 and ADR-0083 §D5
now pin:

```rust
pub enum DriverError {
    StartRejected { failure: DriverStartFailure },
    // existing non-start variants unchanged
}

pub struct DriverStartFailure {
    pub class: DriverStartClass,
    pub detail: String,
}

pub enum DriverStartClass {
    Exec(ExecStartFailure),
    Vm(VmStartFailure),
    Unclassified { driver: DriverType },
}

impl From<&DriverStartFailure> for TransitionReason;
```

The nested Exec and VM variants/fields are exactly those in ADR-0083 §D5.
`failure.detail` is non-empty verbatim low-level text and remains
`AllocStatusRow.detail`; no consumer may classify from its spelling. The
conversion is pure and total. `Unclassified` maps to the existing
`DriverInternalError { detail }` and is the only unknown fallback.

**Exec preservation.** The three live binary classifications remain byte- and
meaning-compatible: ENOENT → `ExecBinaryNotFound`, EACCES →
`ExecPermissionDenied`, ENOEXEC → `ExecBinaryInvalid { kind:
"exec_format_error" }`. Cgroup setup retains `kind == "create_scope" |
"place_pid"`. The driver selects these from structured OS error identity; the
action shim no longer owns an Exec prefix table. Existing verbatim detail is
preserved.

**VM/VMM field preservation.** ADR-0082 D1.1 pins structured `VmmError`
variants and adds `VmProcess.diagnostics: VmmDiagnostics`. Its pure
`console_tail()` snapshot and final `VmmExit.stderr_tail` read the same bounded
capture. `VmDriver` owns the structural join from VMM, per-allocation artifact
verification, boot clock, `VmmExit`, beacon/EXEC, and VM-local storage facts
into `VmStartFailure`. `VmmProbeError` remains composition-time startup refusal
and never becomes an allocation cause. The already-known post-READY EXEC write
failure receives append-only row 15,
`VmGuestCommandDispatchFailed { detail }`; mid-run rows 13/14 remain outside
`DriverStartFailure`.

**Reuse and effect-contract analysis.** No subsystem or service is added. The
existing Driver/VMM/action-shim boundaries are extended; each new value exists
only because no current type can carry the required contract without making an
invalid state representable (`TransitionReason` is too broad for a start-error
port; terminal-only `VmmExit` cannot supply a live deadline snapshot).

| Component / boundary | Reuse or no-existing-alternative ruling | Contract shape | Aggregate-bounded universe | Declared delta | Restricted capabilities | Assertion mechanism |
|---|---|---|---|---|---|---|
| `DriverStartFailure` + `From<&DriverStartFailure>` | **New value; no existing alternative.** Reusing `TransitionReason` admits healthy/reconciler-only variants; reusing `String` loses fields. | pure-function / return-only | One `DriverStartFailure`; closed nested enum set in ADR-0083 §D5 | Return exactly one `TransitionReason`; mutate nothing | None | Rust exhaustive match + complete variant-table test |
| `ExecDriver::start` classification edge | Reuse existing ExecDriver; replace only its string-flattening constructors | bounded-change | One allocation's cgroup scope, child process, supervision entry and emitted start result | Existing success delta unchanged; on rejection return one typed Exec class + verbatim detail; no additional surviving process/scope | Existing `CgroupFs`, `Clock`, and process-spawn boundary only | Existing Exec acceptance outputs unchanged; fault cases vary diagnostic wording while holding OS error identity fixed |
| `VmDriver::start` | Reuse existing VmDriver and three-way race; extend fact preservation | bounded-change | One allocation's configured kernel/rootfs paths, run dir, beacon listener/session, cgroup scope, VMM process, diagnostics handle, live-allocation claim | Success delta unchanged; each failure returns one typed VM/unclassified cause and performs the already-required cleanup to no surviving VMM/scope/run-dir/clone | Required `Vmm`, `Clock`, `CgroupFs`, `CgroupAccounting`, and typed `VmHostLayout`; no global OS object is passed into core | S-VM-33…37/41 plus component fault matrix; cleanup assertions on every non-Ok arm |
| Per-allocation kernel/rootfs verification inside `VmDriver::start` | Reuse `KernelImage::validate` and configured `RootfsPlan`; no second validator | bounded-change with empty mutation set | Exactly the configured kernel path's bounded magic window and configured rootfs-master metadata for one allocation | **∅** — reads only; neither path nor surrounding directory may change | The adapter-host filesystem read capability already owned by VmDriver; no write surface is exposed to the pure validator | Before/after metadata/content identity for the two fixture paths; TOCTOU S-VM-33/34/41 proves read timing |
| `CloudHypervisorVmm::create` | Reuse existing adapter; widen `VmmError` and return value only | bounded-change | One per-launch rootfs clone, one CH process, one exit watch, one bounded diagnostics capture, and the configured run-dir/API-socket names | Success creates exactly one clone/process/watch/capture; failure leaves no process or clone and returns one typed `VmmError` | Typed `VmConfig`, host process spawn, filesystem clone, and bounded stderr capture; probe remains mandatory before use | Existing VMM equivalence + fault catalogue + real adapter probe; live/final tail coherence assertion |
| `SimVmm::create` | Reuse existing simulation adapter; no production shape concession | bounded-change | One scripted VM process/watch/capture in simulator state | Mirror the host adapter's success/failure observations without real I/O | Injected script only | Same VMM equivalence sequence as host adapter |
| `VmmDiagnostics` read handle (`console_tail`) | **New value; no existing alternative.** `VmmExit` is available only after termination and cannot satisfy the deadline arm. | pure-function / return-only | One process's private bounded capture | Return a snapshot; mutate nothing and perform no I/O | Read handle only — the sole diagnostics capability `VmProcess` carries, with no append surface | Repeated-read stability + final `VmmExit.stderr_tail` coherence |
| `VmmDiagnosticsWriter::append` (ADR-0082 §D1.1, amendment 2026-08-16) | **New capability; no existing alternative.** Cross-crate `CloudHypervisorVmm` / `SimVmm` must populate the capture, and the reader deliberately exposes no write surface. | bounded-change | Exactly the same one capture its paired reader observes | Retained tail advances under the line/byte bound; no second storage and no other observable effect | Sole non-`Clone`, non-`Sync` writer, held only by the adapter's capture task | Bound + lossy-UTF-8 unit tests; host/sim equivalence on live-vs-final tail |
| `VmmError → DriverStartFailure` join in `VmDriver` | Reuse existing core error boundary; make its mapping structural | pure-function / return-only for the mapping itself | One `VmmError` plus the already-known driver context | Return one typed VM/unclassified failure; mutate nothing | None beyond typed inputs | Exhaustive mapping table; changing `Display` text cannot change class |
| Action-shim start/restart rejection arm | Reuse ADR-0023 writer; delete classifier responsibility | bounded-change | One target allocation row and its one corresponding lifecycle broadcast event | Write `state: Failed`, the converted reason, and the exact verbatim detail once; no other allocation row changes | Existing `DriverRegistry` and observation/event writer interfaces only | Initial-start + restart-start acceptance equality; row/event reason byte equality |
| `xtask::dst_lint` retired-shape guard | Reuse existing AST lint; no new enforcement tool | pure-function / return-only over loaded source text | Driver trait and action-shim source ASTs | Emit diagnostics only; source delta **∅** | Read-only source input | Gold tests reject `reason: String` and `classify_driver_failure`, accept the typed shape |

Every bounded-change row declares its complete mutation universe and exact
delta; everything outside that universe is preserved. The composition-root
invariant remains **wire → probe → use** for both host and sim VMM adapters. The
three enforcement layers are orthogonal: Rust types/exhaustiveness establish
shape, the AST lint rejects regression to the retired API, and behavioral
fault/acceptance tests establish real-environment semantics.

**Scenario reconciliation — all S-VM-33…41 remain and no ID moves.**

- **S-VM-33** keeps `VmKernelNotFound { path }`, but its Given is corrected to
  valid-at-composition then delete-before-this-start.
- **S-VM-34** remains the missing-rootfs vertical proof and becomes the first
  end-to-end consumer of the typed transport.
- **S-VM-35** retains its existing hypervisor-binary TOCTOU shape.
- **S-VM-36** receives the real deadline milliseconds and live console tail;
  it remains the KVM-required failure in the named-cause group.
- **S-VM-37** uses `DriverStartClass::Unclassified { driver: Vm }` and the
  existing `DriverInternalError` result; no new catch-all variant is minted.
- **S-VM-41** keeps the format diagnosis, but its Given is corrected to
  valid-at-composition then replace-before-this-start.
- **S-VM-38** remains the pre-scheduling `[service]+[vm]` rejection and is
  technically independent of the typed-error chain.
- **S-VM-39** retains acceptance/scheduling and now closes Slice-02's existing
  "accepted and run" clause by requiring the VM allocation to reach Running.
- **S-VM-40** does the same when its first scheduled firing becomes due; it is
  therefore reclassified `@requires-kvm`. The current count becomes 46 of 88.

The two TOCTOU corrections change only reachability, never expected operator
outcome. The S-VM-39/40 additions close an already-present Slice-vs-DISTILL gap;
no scenario is removed, weakened, renumbered, or replaced.

**DELIVER re-decomposition — execution history remains immutable.**

| Step | Current disposition | Scope / scenarios | Dependency |
|---|---|---|---|
| `03-01` | Completed checkpoint only | Records the pre-RED blocker; owns no test or S-VM scenario | historical `01-08` |
| `03-05` | New upstream-contract vertical slice | Exact typed API across core/Exec/Vm/VMM/shim plus the real S-VM-34 rootfs `serve + deploy` proof | `01-08` |
| `03-06` | New remaining named-cause delivery | S-VM-33, 35, 36, 41 | `03-05` |
| `03-02` | Revised, ID preserved | S-VM-37 typed unknown fallthrough | `03-06` |
| `03-03` | Revised, ID preserved | S-VM-38; independent semantic rejection | `01-08` |
| `03-04` | Revised, ID preserved | S-VM-39/40 accepted, scheduled, and run | `03-03` |

03-05 owns an operator-visible S-VM-34 proof rather than landing a horizontal
mechanism with only component tests. 03-06 then extends the same production
path. Pending IDs 03-02…04 are not repurposed or renumbered. The phase gains
two steps (32 → 34 total); `.develop-progress.json` keeps all 15 completed IDs,
keeps old 03-01 completed, and inserts 03-05/03-06 ahead of the remaining
phase-03 work. `execution-log.json` and every history artifact remain untouched.

**Rejected alternatives.** (1) Directly carry `TransitionReason` in
`StartRejected`: allows healthy and reconciler-only reasons at a driver-failure
boundary. (2) Add a VM text parser beside the Exec parser: still cannot recover
discarded fields and binds correctness to upstream prose. (3) Mutate completed
03-01's execution record into a success: falsifies what checkpoint `3222f030`
actually recorded. (4) Land a no-scenario typed-transport step: violates the
production-entry vertical-slice rule; S-VM-34 is the thin live loop.

**Files touched by this entry.** `docs/product/architecture/adr-0032-…md`,
`adr-0082-…md`, `adr-0083-…md`; this `wave-decisions.md` entry and Changelog;
`distill/test-scenarios.md`; `slices/slice-02-boot-failure-vocabulary.md`;
`deliver/roadmap.json`; and `deliver/.develop-progress.json`. No `brief.md`,
Rust source, test source, `CLAUDE.md`, execution log, history, or issue is
touched or created.

---

### DWD-25 — The `[vm]` spec's own `kernel`/`rootfs` become the artifact contract; the node-level `vm_artifacts` seam is deleted and VM composition goes unconditional (2026-08-17, DESIGN ruling — Morgan, GH #42)

**Recorded into the DISTILL log by the DESIGN wave (`nw-solution-architect`)
per explicit dispatch. Docs and roadmap only — no Rust source, test source,
execution log, history file, or GitHub issue is touched or created.**

**The finding, executed rather than asserted.** Verification expectation
`E06-vm-job-deploy-reaches-running` drove a **default-features** `overdrive`
binary on a real x86_64 + KVM host (SHA `655ac964`, `SEED=1`). Sub-claim 0
(the box can boot a guest) and sub-claim 1 (`deploy` exits 0, prints
`Accepted.`) pass. Sub-claims 2 and 3 are **refuted**: the allocation sits
`Failed` for all 45 polls and `resource_delta.txt` reads
`new_hypervisors=0 new_run_dirs=0 new_scopes=0`. The operator surface names
the cause itself:

```
    reason: driver internal error: no vm driver composed on this node
```

KPI **K4** — "the production composition path can reach the VM driver via
`overdrive serve` + `overdrive deploy`… with **no** test-only wiring", the
feature's own binary pass/fail bar (`feature-delta.md:2398`, instrumented by
this catalogue at `:2427`) — therefore reads **NOT MET**. This is the
precedent the feature's risk register named in advance at
`feature-delta.md:2445`: *"the mechanism composes but no production path
reaches it."*

**Investigation (from the source, not assumed).** Five facts, each verified:

1. `ServerConfig.vm_artifacts: Option<VmBootArtifacts>` is
   `#[cfg(feature = "integration-tests")]`, as are `VmBootArtifacts` itself,
   `compose_vm_driver`, the composition block guarded by `if let
   Some(artifacts) = config.vm_artifacts.clone()`, and the two `serve`
   entrypoints that set it (`run_with_dataplane_and_vm_artifacts`,
   `run_with_vm_artifacts`). `main.rs` calls the production
   `serve::run(args)`, which leaves the field unset. So
   `vm_artifacts = Some(_)` is **a state only a test seam can produce**.
2. `ServeArgs` carries `bind`, `data_dir`, `config_dir` and nothing else; the
   clap `Serve` variant exposes `--bind` and `--data-dir`. The only config
   file in the CLI is the ADR-0019 trust triple at
   `<config_dir>/.overdrive/config`, whose schema is
   `{current-context, contexts[].{name,endpoint,ca,crt,key}}` — a TLS identity
   artifact with no `ServerConfig` relationship whatsoever.
3. **The per-allocation artifact surface already exists and is already
   ratified.** ADR-0083 § D3 declares `VmPayload.kernel: PathBuf` and
   `.rootfs: PathBuf`, both annotated `// operator surface, BYO artifact`;
   § D4's `[vm]` block carries both keys; the 2026-08-12 amendment persists
   them in the `V2` envelope.
4. **Those values are carried faithfully all the way to the driver and then
   ignored.** `[vm]` TOML → `VmInput` (`deny_unknown_fields`) → wire DTO →
   `JobV2::from_submit` → rkyv `Vm { command, args, kernel, rootfs }` →
   `WorkloadLifecycle` (both the `StartAllocation` and `RestartAllocation`
   arms) → `DriverPayload::Vm(VmPayload { kernel: PathBuf::from(kernel),
   rootfs: PathBuf::from(rootfs), .. })` → action shim passes `spec` through
   unmodified → `driver.start(&spec)`. But `VmDriver::provision_vmm` reads
   `self.layout.kernel` and `self.layout.rootfs_master`, and `spec.driver` is
   touched exactly twice in the whole file — at the `.command()` / `.args()`
   accessors — and is **never** pattern-matched into its `Vm` arm.
5. E06's runner already deploys a spec whose `[vm]` block names `kernel` and
   `rootfs`. It was authored against the per-allocation shape.

So the platform has two candidate artifact contracts: one ratified by ADR-0083
and fully plumbed but unread, and one implemented but reachable only from a
test seam. **The gap is not a missing config surface. It is a consumer reading
the wrong source.**

**Ruling — per-allocation. No new operator surface is created; four items are
deleted.** Pinned in full as ADR-0083's 2026-08-17 amendment (§§ D3a–D3e) with
the `VmHostLayout` / `KernelImage::validate` consequences in ADR-0082's
2026-08-17 amendment. In short:

- `VmDriver::provision_vmm` binds `let DriverPayload::Vm(payload) =
  &spec.driver else { … }` and uses `payload.kernel` / `payload.rootfs`. No
  accessor is added to `DriverPayload` — `VmPayload`'s fields are already
  `pub`. A non-`Vm` payload takes the existing
  `DriverStartClass::Unclassified { driver: DriverType::Vm }` fallback.
- `VmHostLayout` sheds exactly two fields (`kernel`, `rootfs_master`); its own
  "single fixed template per node" doc comment becomes false and is corrected
  in the same commit.
- `VmBootArtifacts`, `ServerConfig.vm_artifacts`,
  `run_with_dataplane_and_vm_artifacts` and `run_with_vm_artifacts` are
  **deleted, not ungated** — with artifacts arriving per allocation there is
  no node-level artifact to configure, so the seam has no production
  counterpart to promote. Test callers move the paths into the `[vm]` spec
  they deploy, which is what an operator does.
- `vmm_override` and `run_with_dataplane_and_vmm_override` **stay gated**:
  ADR-0083 § D8's adapter-substitution fault seam is a genuine test-only
  capability with no production analogue, and is untouched.
- `compose_vm_driver` and its call site lose the `#[cfg]` and the `if let`;
  composition is discover → probe → insert, gated only by `Vmm::probe`.

**Why not node-level `serve` flags.** Considered and rejected on four counts,
recorded in the ADR: it contradicts ADR-0083 § D3 and would make the platform
silently ignore what the operator wrote in the spec; it is strictly *larger*
(a clap surface + `ServeArgs` field + `main.rs` plumbing + validation, while
*keeping* all four deleted items); it does not survive GH #259, which resolves
per-workload images that a node-wide template cannot express; and it would
require rewriting E06's runner to match the implementation — self-assessment
of the kind `.claude/rules/verification.md` § Enforcement rejects. Under the
ruling, E06 re-runs **unchanged**.

**Earned Trust is preserved, not traded away.** The *hypervisor capability* is
still proven once at boot by `Vmm::probe` (reflink, binary, `/dev/kvm`, run
dir) — that is what "prove it once, use it many times" was always about. The
*artifact* is proven per allocation by `preflight_kernel`, which already
re-reads the path and re-runs `KernelImage::validate` immediately before
`Vmm::create` and already calls itself a "Per-allocation kernel preflight".
Only the redundant boot-time validation of a node-wide path disappears, with
the path itself. `VmStartFailure::{KernelNotFound, RootfsNotFound,
KernelFormatUnsupported}` keep naming the exact path — now the one the
operator actually wrote.

**Capability absence — reuse the typed contract, mint nothing.** A node whose
`Vmm::probe` fails still boots with no `Vm` entry (absence stays a first-class
answer). The dispatch-time registry-miss keeps
`DriverStartClass::Unclassified { driver }` — the action shim "owns
persistence only" and must carry no per-driver branch — and its `detail`
becomes operator-actionable, naming the capability and pointing at the boot
log's `driver.vm.not_composed` reason. Per DWD-24, `detail` is free-form
verbatim text and **never** a classification input, so no contract changes.
**No new `TransitionReason` variant** because: the registry miss is
driver-kind-generic (a `Vm`-prefixed variant would be the wrong shape, a
generic one duplicates `DriverInternalError`); reusing
`VmStartFailure::HypervisorAbsent { searched }` would force the shim to
synthesise driver-specific knowledge it does not have; and the properly typed
answer is the **admission-time** rejection ADR-0083 § D2 already designs,
which DWD-23 is the record of. **This entry does not build the admission-time
gate and promises nothing about when it is built** — it is out of scope, and
no forward pointer is written in its place. DWD-23 recorded "no GitHub issue
created — follow-up surfaced for approval"; that remains true, no number is
invented, and none is implied (CLAUDE.md § "Deferrals require GitHub issues").

**Supersession, stated plainly.** `[vm] kernel` / `[vm] rootfs` are a slicing
mechanism, not a product commitment (user ruling 2026-08-11,
`feature-delta.md:4229`). GH
[#259](https://github.com/overdrive-sh/overdrive/issues/259) deletes both keys
and replaces them with an image reference; the factory then resolves that
reference into host paths and fills the same `VmPayload` fields, leaving
`VmDriver` unchanged. One cut at one boundary — available only because
artifacts are per-allocation.

**Adjacent-doc and call-site fallout, named rather than left to rot.** Steps
03-05 and 03-06 landed S-VM-33/34/35/36/41 against "the path `serve` composed
against". All five are unchanged in substance and no ID moves; their fixtures
must now mutate the path the *spec* names, and the shared
`VmBootArtifacts`-taking `spawn_vm_server` helper disappears. Deleting
`VmBootArtifacts` breaks **every** call site that names it — roughly thirty
across `vm_walking_skeleton.rs`, `vm_boot_failure_vocabulary.rs` and
`vm_reclamation_tier3.rs`, **plus one outside the VM test files entirely**,
`overdrive-control-plane/tests/integration/workload_lifecycle/
convergence_loop_spawned_in_production_boot.rs` (`vm_artifacts: None`), which
would otherwise be an unscoped compile break. Step 03-07 owns all of it
(CLAUDE.md § "Behavior change must mark stale adjacent docs").

**Two residuals recorded honestly, neither solved here.** (1) `Vmm::probe`'s
reflink check runs against the adapter's own `image_dir` (`/srv/vm`), while
the per-launch `FICLONE` now targets the *operator's* rootfs directory —
`FICLONE` is intra-filesystem, so the boot probe no longer proves the clone
will succeed, and today that failure renders as an *unclassified*
internal-shaped error. S-VM-94 already owns the fail-closed behaviour and its
target becomes the operator-named path; **no `VmStartFailure` variant is
minted here** (new `core` API surface, and the right moment to type it is when
S-VM-94 is implemented). **E06 cannot catch this** — it stages under
`/srv/vm/overdrive-testing`, beneath the probe's own default. (2) The
per-launch clone is now written into an operator-chosen directory, which may
be read-only or shared; no operator surface exists to redirect it. Both are in
ADR-0083's Consequences.

**Ordering is encoded in the graph, not asserted in prose.** Phases 04/05/06
have **not** executed and both 04-01 and 05-02 edit `vm_driver.rs`, the file
03-07 restructures. The leading step of each — 04-01, 05-01, 06-01 — therefore
takes an explicit dependency on 03-07, so an orchestrator reading
`roadmap.json` cannot start them first.

**Scenarios.** Two added, taking the two lowest genuinely-unused IDs per this
file's established gap-reuse practice (the same rule DWD-14 applied when it
filled gap 41): **S-VM-54** (a configured-by-spec VM job runs end to end
through the shipped binary — the in-tree companion to E06) and **S-VM-82** (a
node without the hypervisor capability reports what is absent, actionably).
Count 88 → 90. No scenario removed, renumbered, or re-tagged.

**Roadmap.** Two steps appended to phase 03 as **03-07** and **03-08** — every
phase-03 step has already executed, so nothing is reordered and no execution
history is rewritten. Both must precede phases 04/05/06, whose behaviour is
otherwise unverifiable through production. Phase 03's display name widens to
name the second concern; no dependency references a phase name.

**Files touched by this entry.** `docs/product/architecture/adr-0082-…md`
(2026-08-17 amendment), `adr-0083-…md` (2026-08-17 amendment §§ D3a–D3e); this
`wave-decisions.md` entry and Changelog; `distill/test-scenarios.md`
(S-VM-54, S-VM-82); `feature-delta.md` (§ *Wave: DELIVER / [WHY] Upstream
Issues*); `deliver/roadmap.json` (steps 03-07, 03-08; phase-03 name). No
`brief.md`, Rust source, test source, `CLAUDE.md`, execution log, progress
file, or issue is touched or created. No commit made by this pass.

---

## Changelog

- 2026-08-11 — Initial DISTILL wave decisions captured. 0 contradictions in reconciliation (both the orchestrator's pre-verified summary and this session's independent full read agree). 74 scenarios across 9 user stories + 1 cross-cutting reconciler + 3 port-contract-enforcement scenarios, tagged and traced to all 10 KPIs. Walking skeleton: S-VM-01, one scenario, Slice 01. Adapter strategy: this project's four-tier model (Tier 1 in-memory default lane / Tier 3 real-Lima `integration-tests` lane), with `Sim*` fault injection at the port boundary for substrate-lie scenarios. Mandate 7 scaffolding: scoped to Slice 01 + three cross-cutting pure-function scenarios (15 scaffolds, verified compiling and RED by execution — `cargo check`, `cargo clippy -D warnings`, `cargo nextest run`, all clean); the remaining 59 scenarios' scaffolds are deferred to DELIVER's per-slice RED phase with exact file placement already committed in DWD-04. Two drafting corrections made and recorded (DWD-07): the no-subprocess CLI convention, and three dangling scenario references closed.
- 2026-08-11 — Peer review (Sentinel, `nw-acceptance-designer-reviewer`): NEEDS_REVISION (1 blocker, 2 high). Fixed, none waived (DWD-08): added the mandate-14 `@contract-shape:` tag to all 74 scenarios; corrected a 2-scenario undercount surfaced by the tagging pass (72 → 74, mechanically recounted) across `test-scenarios.md`, this file, and `feature-delta.md`'s DISTILL section; confirmed the deferred-scaffold forward-reference table (DWD-04) already satisfies the reviewer's other high finding.
- 2026-08-11 — Second-round adversarial review (Sentinel + Atlas, two independent fable dispatches, both `needs_revision`): FIXED (DWD-11). One BLOCKER (S-VM-88/89 phantom references + the third undefined §105a.11 invariant) — three scenarios defined under a new AC-20 (S-VM-87, 88, 89). Four systemic HIGH findings (the NEW-1 pins under-covered) — four scenarios under a new AC-19 (S-VM-77…80). Eight more HIGH findings — S-VM-81 (fourth evaluation), S-VM-93 (`CgroupAccounting` equivalence), S-VM-94 (per-launch `FICLONE`), S-VM-74 (`MtlsInterceptWorker` gating), S-VM-76 (`VmDriver::stop` totality, new AC-18, with a documented Driving Ports table carve-out), S-VM-13 narrowed + S-VM-75 added (non-reflink envelope-claim fix), S-VM-35 rewritten (TOCTOU, fixes the S-VM-12 contradiction), S-VM-49 reworded (fixes the S-VM-53 contradiction). Nine MEDIUM/LOW findings — S-VM-26/S-VM-20/S-VM-08/S-VM-44/S-VM-37 corrected in place; DWD-03/DWD-06 accounting errors fixed; a dst-lint-clause AC-ownership decision (DWD-09) and a kernel-matrix-ownership decision (DWD-10) recorded. One item SETTLED by explicit user ruling, not fixed: DWD-06a records that `.claude/rules/testing.md` governs over the generic skill's ADR-025 statement, so the scaffold deferral stands; ownership of per-slice scaffold authorship (crafter) and review (`nw-software-crafter-reviewer`) answered. Two items marked BLOCKED on the concurrent DESIGN pass, not guessed: S-VM-65's mid-run storage-daemon-death `TransitionReason` variant, and the `SimVmm`/`SimVmHostState` production-composition-root injection seam for S-VM-13/51/67. Scenario count 74 → 87; error/edge coverage 59% → 60%; zero dangling `S-VM-N` references (mechanically re-verified across all three artifacts).
- 2026-08-11 — Concurrent DESIGN pass ruled on both outstanding blockers (DWD-12). **RESOLVED**: S-VM-65's mid-run storage-daemon-death variant — ADR-0083 §D5 gained row 14 (`TransitionReason::VmStorageDaemonDied`), checked ahead of `ExitKind` entirely; S-VM-65 rewritten with a second scenario shape (guest self-reports `EXIT 0` after the daemon dies) that fails if the precedence ordering is wrong. **RESOLVED**: the `SimVmm` injection seam for S-VM-13/S-VM-51 — ADR-0083 §D8, `ServerConfig.vmm_override`, a whole-port substitution shaped after `mtls_identity_override`, not `dataplane_override` (rejected by name, §A10); both scenarios' crafter notes now name the seam and gating exactly. **STAYS BLOCKED, precisely**: S-VM-67 — ADR-0083 §D8 explicitly rules it outside the seam's reach (no `Vmm` method sits downstream of virtiofsd's sandbox check; no storage-daemon supervision port exists); its crafter note is corrected to state this is a scoping decision, not a missing seam name, and names the two candidate paths without choosing either. Upstream Issues reduced from two blocked items (four blocked scenario references) to one open item. Adapter Coverage Table's `Vmm (SimVmm)` row and Self-Review Checklist item 4 corrected to drop S-VM-67 (never covered by this seam). Scenario count unchanged at 87 (mechanically re-verified); no ADR, `brief.md`, or Rust file touched.
- 2026-08-11 — User ruling closes the last open item (DWD-13). **RESOLVED**: S-VM-67 — path (b) chosen: `[D8d]`'s `--sandbox=namespace`-unavailable case is verified at the launch-argument construction layer (private fields, one rendering site, a pure unit test on the rendered value — the same enforcement tier ADR-0082 §D2.1 already uses for `image_type=raw`), never through a real `overdrive serve`. **This feature mints no storage-daemon supervision port.** S-VM-67 rewritten in full: `@tier3`/`@real-io` → `@tier1`/`@in-memory`, `@contract-shape:bounded-change` → `@contract-shape:pure-function`, `@property` gained (mirrors S-VM-17's pure-function-plus-`@error_path` precedent, `@error_path` retained), driving port changed from `overdrive deploy` to the storage daemon's launch-argument rendering site (a not-yet-ADR-pinned Slice 04 type — DELIVER's own naming, per CLAUDE.md § "Implement to the design"). The scenario's `Then` now carries an explicit boundary statement: it proves only what argument the rendering function constructs, never that a running `virtiofsd` enforces it or that the platform genuinely fails closed end-to-end — both stay an undischarged Tier-3 property of Slice 04. No separate Tier-3 runtime-half scenario was added (reasoned in DWD-13: no port to inject through, no genuinely-lying host in the one-kernel Lima envelope, and minting either now would invent API surface past the design). Sibling references corrected: the `@real-io` Adapter Coverage Table's virtiofsd row, the US-VM-9 AC-to-Scenario Traceability row, Self-Review Checklist item 4 (all three previously touched by DWD-12 for the S-VM-13/S-VM-51 resolution, now re-verified against S-VM-67's new resolution), plus two references DWD-12 did not reach: the top-of-file Driving Ports table's `overdrive deploy` row (range corrected to exclude S-VM-67; a new row added for the pure-function driving port) and this file's own DWD-04 crate-placement table (same range correction). Error/Edge Path Coverage counts updated: `@property` 20 → 21, `@tier3`/`@real-io` 61 → 60, `@tier1`/`@in-memory` 29 → 30; `@error_path` unchanged at 40; total unchanged at 87; error+edge coverage unchanged at 60%. Upstream Issues now shows **zero** open items; all three resolved items (S-VM-65, the S-VM-13/S-VM-51 seam, S-VM-67) kept struck-through for the audit trail. No ADR, `brief.md`, or Rust file touched by this DISTILL pass (the ADR amendments already landed via the concurrent DESIGN pass before this pass started). No GitHub issue created; #259–#263 remain the only real numbers in scope, #264 closed. No commit made.
- 2026-08-11 — AC-09 completeness gap closed, found by a fable review cross-checking the concurrent `deliver/roadmap.json` pass against ADR-0083 §D5 (DWD-14). ADR-0083 §D5 pins **five** Slice-02 Cause variants; the roadmap's Slice-02 step 03-01 criteria enumerated only four, and `test-scenarios.md` had **zero** entry for row 5 (`VmKernelFormatUnsupported { path, arch, detail }`) among the original 87 — verified directly (`grep -rn "VmKernelFormatUnsupported" distill/` returned nothing outside `slices/slice-02-boot-failure-vocabulary.md`'s own prose) before acting. **Fixed**: S-VM-41 added — the classification-join half of C-7, companion to S-VM-17's already-proven pure-function half (`KernelImage::validate`), not a duplicate of it; asserts the operator-visible `TransitionReason::VmKernelFormatUnsupported` reads as a format problem, never CH's misleading size-cap/`UefiTooBig` framing. `@contract-shape:bounded-change` `@error_path` `@ac-09` `@tier3` `@real-io` `@correction:C-7`, placed at `crates/overdrive-cli/tests/integration/vm_boot_failure_vocabulary.rs` alongside S-VM-33…37 (already covered by DWD-04's existing `S-VM-33…66` span; no range edit needed). Scenario ID chosen as the lowest genuinely-unused gap (41) rather than extending past 94, matching this file's established gap-reuse practice. Scenario count 87 → 88; `@error_path` 40 → 41; `@contract-shape:bounded-change` 65 → 66; error+edge coverage unchanged at ≈60% (53/88). KPI Traceability K3 row, AC-to-Scenario Traceability US-VM-2 row, and Self-Review Checklist items 8/13/15 updated. Mechanical recount also surfaced and corrected a pre-existing, unrelated off-by-one in Self-Review Checklist item 13's pure-function/bounded-change split (12/65 was already true before this pass, not the claimed 11/66 — both wrong numbers happened to still sum to 87). No ADR, `brief.md`, or Rust file touched; `deliver/roadmap.json` not touched (owned by the concurrent roadmap pass, which cites S-VM-41 by the ID recorded here). No GitHub issue created; #259–#263 remain the only real numbers in scope, #264 closed. No commit made by this pass.
- 2026-08-11 — Iteration-2 review LOW fixed: S-VM-06/07/62's driving-port lines corrected (DWD-15). Prose said `overdrive deploy` (in-process CLI handler) while DWD-04 places all three at `overdrive-core`'s parse-boundary file — `overdrive-core` cannot dev-depend on `overdrive-cli`, so the two statements were mutually exclusive. Verified against the existing compiled scaffold (`vm_spec_driver_table_dispatch.rs`, one of DWD-06's fifteen) and ADR-0083's pinned function (`WorkloadSpecInput::from_toml_str`, `workload_spec.rs:710`) before fixing. Rewrote all three lines to `` `WorkloadSpecInput::from_toml_str()` (pure function — in-process TOML parse boundary, no subprocess, no `overdrive serve` needed) ``; Gherkin, tags, tier, and ACs untouched. Sweep for the same defect class run over all 88 scenarios: found S-VM-77/S-VM-79 naming `overdrive-control-plane`-resident driving ports (`worker/exit_observer.rs`'s loop body; `execute_reclaim_allocation`) against DWD-04's `overdrive-core` placement for S-VM-77…80 — **not fixed**, no scaffold exists as ground truth and the correct resolution (move the crate cell vs. rewrite the driving-port prose) is a placement judgment call outside this entry's prose-correction scope; flagged for DELIVER's AC-19 scaffolding. Also noted, not fixed: S-VM-09/S-VM-19 have no DWD-04 placement at all (an omission, not a contradiction). Scenario count re-verified unchanged at 88. No ADR, `brief.md`, `deliver/roadmap.json`, or Rust file touched. No GitHub issue created; #259–#263 remain the only real numbers in scope, #264 closed. No commit made by this pass.
- 2026-08-11 — Both placement gaps DWD-15 flagged (and declined to fix inline) resolved (DWD-16). **Gap 1 — S-VM-77/S-VM-79**: verified a genuine contradiction, not staleness — `worker/exit_observer.rs` and the `ReclaimAllocation` executor (`execute_reclaim_allocation`, by this crate's existing `action_shim/`-per-action-executor convention) are both `overdrive-control-plane`-resident, and `overdrive-control-plane` depends on `overdrive-core`, never the reverse (Cargo.toml dependency graph checked both directions). Remedy: moved, not rewritten — DWD-04's AC-19 row split, S-VM-78/80 stay at `overdrive-core`, a new row places S-VM-77/79 at `overdrive-control-plane` (`tests/acceptance/vm_reclamation_claim_lifecycle.rs`, NEW file, default lane, matching their existing `@tier1 @in-memory` tags — no tier change). Fabricating a core-side seam was explicitly rejected as an option, per the dispatch's instruction. `test-scenarios.md`'s own top-of-file Driving Ports table carried the identical contradiction independently (its `plan_reclamation` row's exercises column wrongly listed S-VM-77…80) and was corrected the same way, plus a new row added for the `overdrive-control-plane` driving ports. **Gap 2 — S-VM-09/S-VM-19**: confirmed an omission, not a contradiction — both scenarios' own driving-port lines and tags (`@tier3 @real-io`) already match the Tier-3 CLI-driven row's other members exactly; the row's span notation simply never named the two IDs sitting in its own sub-range gaps (06-10, 16-32). Remedy: extended the existing row's span to `S-VM-01…05, 09, 11…15, 19, 33…66, 68` (explicit-inclusion, mirroring DWD-13's explicit-exclusion of S-VM-67 from the same row) rather than a blanket range widen, which would have wrongly swept in S-VM-06/07/08/10 (placed elsewhere by DWD-04's other rows). `test-scenarios.md`'s top-of-file Driving Ports table's `overdrive deploy` row carried the identical omission and was corrected the same way. Neither gap required a tier change; nothing was stopped or reported as blocked. No scenario's tier, tags, ACs, or Gherkin changed; no scenario added, removed, or renumbered. Scenario count re-verified mechanically unchanged at 88 (`grep -c '^#### S-VM-'` and `grep -c '^\*\*Tags\*\*:'`, both 88). No ADR, `brief.md`, `deliver/roadmap.json`, or Rust file touched. No GitHub issue created; #259–#263 remain the only real numbers in scope, #264 closed, none newly cited here. No commit made by this pass.
- 2026-08-12 — `@requires-kvm` capability-class gate recorded (DWD-17), closing a decision `spike/findings.md` explicitly deferred to "Slice 01's first integration test" and that neither the original DISTILL pass nor two adversarial review rounds picked up. Walked all 61 `@tier3`/`@real-io` scenarios; classified 45 as requiring a real `cloud-hypervisor` guest-boot attempt (tagged `@requires-kvm` — including three hybrid scenarios, S-VM-21/22/25, where the tag attaches only to the documented Tier-3-companion clause, not the primary `@tier1`/`@in-memory` shape) and 16 as real I/O that never spawns a guest-booting hypervisor (composition-gate probes S-VM-11/12/75, `SimVmm`-injected faults S-VM-13/51, pre-spawn artifact-validation rejections S-VM-33/34/35/41/58/59, an admission-time rejection S-VM-38, generic host-primitive adapter equivalence S-VM-91/93, and S-VM-94 whose own `Then` states no hypervisor process was spawned). Five dispositions recorded as genuinely ambiguous rather than silently resolved: S-VM-04/S-VM-39 (leaned `@requires-kvm`, happy-path/walking-skeleton-adjacent framing), S-VM-40 (leaned NOT — cron-deferred), S-VM-23/S-VM-28/S-VM-30 (leaned `@requires-kvm` — "No Fixture Theater" + Tier-3-companion intent), S-VM-58/S-VM-59 (leaned NOT — pre-spawn-validation analogy), S-VM-91/S-VM-93 (leaned NOT — generic cgroupfs/filesystem primitives). A top-of-file explanatory note added to `test-scenarios.md` citing the spike's measured asymmetry (bare-metal x86_64 12/12 vs nested-aarch64 ~1-in-3). DWD-04's rows annotated with the capability-lane split where a single row/file mixes `@requires-kvm` and non-`@requires-kvm` members (the walking-skeleton/Tier-3-CLI row, the `Vmm`/`VmHostState` adapter-equivalence row, and the `VmReclamation` Tier-3-shapes row, plus seven individually-noted rows in the DWD-11 addendum table). The concrete Rust-side gating mechanism (feature name, preflight shape) is explicitly NOT invented here — it is pinned by the concurrent `deliver/roadmap.json` pass, which this entry asks to reconcile against; not a deferral requiring a GitHub issue, since the mechanism-naming is in-feature work assigned to that concurrent pass. No scenario's tier, ACs, or Gherkin body changed; no scenario added, removed, or renumbered. Scenario count re-verified mechanically unchanged at 88 (`grep -c '^#### S-VM-'` and `grep -c '^\*\*Tags\*\*:'`, both 88). No ADR, `brief.md`, `deliver/roadmap.json`, or Rust file touched. No GitHub issue created; #92, #222, #248, #257, #259–#263 remain the only real numbers in scope, #264 closed, none newly cited here. No commit made by this pass.
- 2026-08-12 — `@requires-kvm` bound explicitly to the `kvm-tests` Cargo feature the concurrent roadmap pass pinned; DWD-17's ten flagged-ambiguous scenario IDs reconciled against the roadmap's file-level gate (DWD-18). Binding stated in both `test-scenarios.md`'s top-of-file `@requires-kvm` note (replacing the now-stale "pinned by the concurrent roadmap pass" forward pointer) and as an addendum inside DWD-17. Naming reasoning recorded: `bare-metal-tests` rejected as false (CI is a GitHub-hosted VM, not bare metal, and the spike measured it trustworthy); the property gating matters is nesting, not hardware presence (the spike's own diagnosis); `kvm-tests` names the lane generically, shaped like `integration-tests`; declaration is narrow (only `overdrive-host`/`overdrive-cli`, since `kvm-tests` gates no mutation-testing surface); the preflight fails loudly rather than silently skipping. Ten-way reconciliation (S-VM-04, 23, 28, 30, 39, 40, 58, 59, 91, 93) checked in the one dangerous direction (leaned `@requires-kvm` but roadmap file NOT gated) — **zero scenarios land in that bucket**: five (S-VM-04/23/28/30/39) are consistent (leaned yes, file gated `kvm-tests`); five (S-VM-40/58/59/91/93) are either harmlessly over-gated (S-VM-40/58/59/91 — leaned NOT, file gated anyway) or correctly ungated (S-VM-93 — leaned NOT, roadmap explicitly states it "never goes through the `Vmm` port... deliberately NOT `kvm-tests`"). No scenario's own classification needed correction; no roadmap file placement flagged as wrong. Confirmed the two deliberately-ungated files agree with their scenarios' tags: `cgroup_accounting_equivalence.rs` (S-VM-93, synthetic cgroup fixtures, never touches `Vmm`) and `vm_storage_daemon_sandbox_arg.rs` (S-VM-67, Tier-1 pure rendering proptest, `@tier1 @in-memory` — never a `@requires-kvm` candidate). No scenario's tier, tags, ACs, or Gherkin body changed; no scenario added, removed, or renumbered. Scenario count re-verified mechanically unchanged at 88 (`grep -c '^#### S-VM-'` and `grep -c '^\*\*Tags\*\*:'`, both 88). `deliver/roadmap.json` read-only, not touched. No ADR or Rust file touched. No GitHub issue created; #92, #222, #248, #257, #259–#263 remain the only real numbers in scope, #264 closed, none newly cited here. No commit made by this pass.
- 2026-08-14 — Mutation-testing cadence reconciled (DWD-19), user-approved. The per-step `cargo xtask mutants` gates named in individual DELIVER step ACs (roadmap step 01-05 AC #4 the worked example) are superseded by one end-of-DELIVER, per-feature whole-phase-diff gate (`cargo xtask mutants --diff origin/main`, kill-rate ≥ 80%), per CLAUDE.md § "Mutation Testing Strategy". Coverage is unchanged — the whole-diff run mutates `reserve_bytes` / `MemoryPlan::cgroup_max_bytes` (in the phase-01 diff) exactly as a per-step `--file` run would; only the cadence differs. The `@mandatory:mutation_target` tags on S-VM-18/S-VM-20 still attach per-step; only the run is deferred. A single one-line pointer was appended to step 01-05's `implementation_notes` in `deliver/roadmap.json`; the `criteria` arrays left unchanged (this wave-decision is the authoritative reconciliation). No scenario added, removed, renumbered, or re-tagged; no `test-scenarios.md`, `brief.md`, or Rust file touched by this entry. No GitHub issue created.
- 2026-08-14 — 01-07 review reconciliations (DWD-20), three items, all rulable without a user scope decision. **Item 1 (PRIMARY)**: the `Driver` post-stop `status() → NotFound` contract (`traits/driver.rs`) was violated by the shipped `VmDriver::stop`, which left the live-map entry `Live`. Ruled (reviewer option (c)): `stop` transitions the entry `Live → EndingInFlight` under the same lock it extracts the live state with, so `status()` (which already maps `EndingInFlight → NotFound`) satisfies the contract synchronously while `live_allocations()` still reports the entry, keeping the authorship claim for `VmReclamation` (`EndingInFlightIsNeverReclaimed`, § 105a.11 — whose wording already presupposes "its stop has been issued" ⇒ `EndingInFlight`). NOT the `ExecDriver` full-removal shape (would reopen NEW-1); NOT a trait carve-out (contract stands, no `driver.rs` docstring edit). Pinned in `brief.md` § 105a.3 (new transition 3b + amendment) and ADR-0082 § D4; the release side (transitions 5/6) stays step 02-02. **Item 2**: ADR § D7's ownership table already assigns the `VmDriver` `EXEC` write to 01-07, but the shipped beacon-win arm omits it (the guest would block forever at 01-08); ruled a scoped 01-07 addendum (not reassignment to 01-08) — roadmap 01-07 gains a fifth criterion, ADR § D7 gains a not-yet-wired amendment naming 01-07 as the wiring step. **Item 3**: ADR § D2.4 gains a "gap 6" note blessing `KernelImage::path` (`config.rs:201`) and `VmExitWatch::new` (`vmm.rs:202`) as sanctioned 01-06 first-implementor surface. Files touched: `adr-0082-…md` (§§ D2.4 / D4 / D7), `brief.md` (§ 105a.3), `deliver/roadmap.json` (step 01-07 criteria), this file. No Rust file touched — the crafter-facing instructions go to the 01-07 review-remediation dispatch. No GitHub issue created.
- 2026-08-14 — Guest vsock provisioning ruled (DWD-21), a DESIGN-wave ruling (Morgan, `nw-solution-architect`) recorded into the DISTILL log per dispatch. Step 01-08's walking skeleton surfaced a spike→ship regression: the 01-03 `overdrive-init` + 01-04 fixture lost the spike's proven vsock module-load, so on the stock `CONFIG_VSOCKETS=m` kernel the fixture stages, the guest's `socket(AF_VSOCK)` fails `EAFNOSUPPORT` and never beacons — `#[ignore]`ing S-VM-01/02/05/74. Ruled (c) reconcile: `overdrive-init` `finit_module`-loads the three staged vsock `.ko` before `connect_beacon` (no-op on a vsock=y kernel) and the 01-04 fixture stages them from the same `uname -r` as the kernel (the spike's proven-12/12 mechanism / the `[D2]` fallback), WHILE the production appliance kernel builds vsock in (ADR-0068 §4). Lands as the 01-08 review-remediation touching `overdrive-init/src/main.rs` + `overdrive-testing/src/vm_fixture.rs` (both added to 01-08's `source_directories`). ADR-0068 gains §4; ADR-0082 §D4 gains a loader/staging amendment; ADR-0083 unaffected; `brief.md` consistent, untouched. `deliver/roadmap.json` step 01-08: one additive criterion + two source files + an `implementation_notes` remediation note. No scenario changed; no GitHub issue created.
- 2026-08-14 — Alloc→driver index MISS disposition ruled (DWD-22), a DESIGN-wave ruling (Morgan, `nw-solution-architect`) recorded into the DISTILL log per dispatch, closing the 01-08 review's MAJOR finding D1. ADR-0083 §D2a(b) pinned a typed `ShimError::UnknownDriverForAlloc { alloc_id }` on an `alloc_drivers` index miss; the shipped `resolve_drivers_for_alloc` (`action_shim/mod.rs:668-678`) instead **broadcasts** the stop/terminal call to every composed driver, and the variant exists nowhere in the tree. Investigated the four consumer arms (`FinalizeFailed` `:1291`, `RestartAllocation` stop-half `:1604`, `StopAllocation` `:1858`/`:1922`) and the regression that forced the fallback (`stable_does_not_stop_probe_supervision.rs` calls `on_alloc_running` directly, leaving the per-boot index empty). Ruled **(a) bless the broadcast, retire the typed-error pin**: the ADR's rationale attacked a strawman (broadcast reaches the *owning* driver — including `VmDriver` — so no orphan is stranded, unlike the "route to `ExecDriver`" fallback it feared), and the typed error taken literally would route a legitimate per-boot miss to nobody and *create* the orphan SD-1 prevents; runtime confirmed safe (every `Driver::stop`/`on_alloc_*` is NotFound-tolerant/no-op). Options (b)/(c) rejected — they reintroduce the orphan on the stop arms. **No code change** — the shipped code and all four call-site comments already match the amended contract; the crafter only confirms the § D2a(b) citations resolve to the amended text, and MUST NOT implement `UnknownDriverForAlloc`. Amended ADR-0083 § D2a(b) + `brief.md` § 104; § D3 / § D2a(c) name no typed error and are untouched. `deliver/roadmap.json` not touched (no code change ⇒ no AC/scope edit). No `test-scenarios.md` or Rust file touched. No GitHub issue created.
- 2026-08-14 — `[vm]` "at admission" capability rejection ruled (DWD-23), a DESIGN-wave ruling (Morgan, `nw-solution-architect`) recorded into the DISTILL log per dispatch, closing the 01-09 review's MEDIUM finding D2. Roadmap step 01-09 AC #1 promised "rejects `[vm]` at admission"; the shipped behavior admits the spec (`Inserted`) and rejects at DISPATCH (`action_shim` `drivers.get(kind) → None → StartRejected → Failed` naming the capability, S-VM-12) — SAFE, but not "at admission," and never in 01-09's `handlers.rs`-excluding scope. Ruled **(b)**: "at admission" stays required DESIGN intent (SD-5, ADR-0083 §§ D1/D2/D4, `brief.md` § 104); the dispatch-time fallback is ratified as a SAFE INTERIM and STAYS as multi-node-ready defense-in-depth; the admission gate (`handlers.rs::submit_workload` → `state.drivers.supports(..)` before `put_if_absent`, a cheap addition since `AppState.drivers` + `DriverRegistry::{supports,kinds}` exist since 01-08) is scoped to a **follow-up step, pending user build-vs-defer approval**. Option (a) (ratify dispatch-time, drop "at admission") rejected — discards deliberate design, contradicts § D4's "the deploy still fails." Reworded step 01-09 AC #1 to shipped behavior + `implementation_notes` scope note; added implementation-status notes to `brief.md` § 104 + ADR-0083 status header; SPECIFIED (not applied — docs-only) the `action_shim`/`error.rs`/`vm_walking_skeleton.rs` "step 01-09" comment corrections. **No GitHub issue created — follow-up surfaced for approval.**
- 2026-08-16 — Phase-03 typed driver-failure upstream resolution ruled (DWD-24). `StartRejected.reason: String` and `classify_driver_failure` are retired in favor of exact `DriverStartFailure` / Exec / VM classes and a pure exhaustive conversion to `TransitionReason`; Exec's observable classes and verbatim detail stay unchanged; unknown VM/VMM failures reuse `DriverInternalError`. Checkpoint `3222f030` is retained honestly as completed old 03-01 with no scenario ownership. Roadmap adds 03-05 (typed contract + S-VM-34 vertical proof) and 03-06 (S-VM-33/35/36/41), then preserves 03-02/03/04 for S-VM-37/38/39/40 with corrected dependencies. S-VM-33/41 receive reachable post-composition TOCTOU Givens; S-VM-39/40 now prove the VM runs, making S-VM-40 the 46th `@requires-kvm` scenario. Total remains 88; no scenario removed or renumbered. No code, test source, execution log/history, issue, or commit.
- 2026-08-17 — Production artifact supply ruled (DWD-25), a DESIGN-wave ruling (Morgan, `nw-solution-architect`) recorded into the DISTILL log per dispatch, closing the K4 gap that verification expectation E06 measured as NOT MET on a real x86_64 + KVM host (SHA `655ac964`: `deploy` accepted, allocation `Failed` with `no vm driver composed on this node`, `new_hypervisors=0`). Root cause is **not** a missing config surface: ADR-0083 § D3's `VmPayload.kernel`/`.rootfs` are already ratified, already `[vm]`-parsed, already `V2`-persisted and already carried intact to `driver.start(&spec)` — `VmDriver::provision_vmm` simply reads `self.layout.*` instead and never matches `spec.driver`'s `Vm` arm. Ruled **per-allocation**: the driver binds `let DriverPayload::Vm(payload) = &spec.driver else { … }` (no new accessor — the fields are already `pub`); `VmHostLayout` sheds `kernel`/`rootfs_master`; `VmBootArtifacts`, `ServerConfig.vm_artifacts`, `run_with_dataplane_and_vm_artifacts` and `run_with_vm_artifacts` are **deleted, not ungated** (with artifacts per-allocation there is no node-level artifact to configure, so the test seam has no production counterpart to promote — CLAUDE.md § "Ground the premise"); `vmm_override` stays gated as ADR-0083 § D8's genuine fault seam; `compose_vm_driver` and its call site go unconditional, gated only by `Vmm::probe`. Node-level `--vm-kernel`/`--vm-rootfs` flags rejected on four counts (contradicts § D3 and would silently ignore the operator's spec; strictly larger; does not survive GH #259's per-workload images; would require rewriting E06's runner to match the implementation). Earned Trust preserved by scope: the hypervisor *capability* stays proven once at boot by `Vmm::probe`, the *artifact* is proven per start by the already-per-allocation `preflight_kernel` → `KernelImage::validate`. Capability absence reuses the DWD-24 typed contract with an actionable `detail` and **mints no `TransitionReason` variant** (the registry miss is driver-kind-generic; `HypervisorAbsent` would force a per-driver branch into the shim; the typed answer is the admission-time gate DWD-23 already scoped). Amended ADR-0082 (2026-08-17) and ADR-0083 (2026-08-17, §§ D3a–D3e, incl. the #259 supersession statement). Scenarios S-VM-54 and S-VM-82 added at the two lowest unused IDs per this file's gap-reuse practice (88 → 90); none removed or renumbered. Roadmap gains steps 03-07/03-08 appended to phase 03 — every phase-03 step has already executed, so nothing is reordered and no execution history is rewritten. Step 03-06's S-VM-33/S-VM-41 fixtures must re-point at the spec-named path; 03-07 owns that, and neither scenario changes in substance. No Rust source, test source, execution log/history, GitHub issue, or commit.
