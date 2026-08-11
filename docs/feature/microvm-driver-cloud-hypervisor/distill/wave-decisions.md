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
| Walking skeleton + all Tier-3 CLI-driven scenarios (S-VM-01…05, 11…15, 33…66, 68) | `overdrive-cli` | `tests/integration/vm_*.rs`, gated `integration-tests` | Real driving port is the CLI handler; mirrors `exec_spec_walking_skeleton.rs`. **Corrected 2026-08-11 (DWD-13): excludes S-VM-67**, which the original range silently swept in — S-VM-67 is no longer Tier-3/CLI-driven, see the DWD-13 addendum below |
| Pure-function `@property` scenarios (`VmConfig` value family, `plan_reclamation`, `SupervisionSet`, vCPU derivation) | `overdrive-core` | `tests/acceptance/vm_*.rs`, default lane | Port-to-port at function scope; no I/O, no Lima needed |
| Parse-boundary rejections (S-VM-06/07/62) | `overdrive-core` | `tests/acceptance/vm_spec_driver_table_dispatch.rs`, default lane | In-process TOML deserializer, no subprocess and no real VMM needed |
| `JobEnvelope` V1→V2 schema evolution (S-VM-10) | `overdrive-core` | `tests/schema_evolution/workload_intent.rs` (EXISTING file, edited, not a new file) | Per the six-step version-bump procedure; `FIXTURE_V1` never touched |
| `Vmm` / `VmHostState` adapter equivalence (S-VM-90/91) | `overdrive-host` | `tests/integration/vmm_equivalence.rs`, `vm_host_state_equivalence.rs`, gated `integration-tests` | Named exactly by the design (ADR-0082 §D6, ADR-0083 §D7) |
| `VmReclamation` reconciler in-memory shapes (S-VM-21 companion, 24, 26, 27, 29) | `overdrive-core` | `tests/acceptance/vm_reclamation_*.rs`, default lane | Pure `reconcile()` driving port |
| `VmReclamation` reconciler Tier-3 shapes (S-VM-21, 22, 23, 25, 28, 30) | `overdrive-cli` | `tests/integration/vm_reclamation_tier3.rs`, gated `integration-tests` | Real `overdrive serve` convergence loop |
| DST invariants (`VmReclamationConverges`, `SupervisedVmSurvivesEveryTick`, `VmReclamationIdempotentSteadyState`, `EndingInFlightIsNeverReclaimed`) | `overdrive-sim` | `src/invariants/vm_reclamation.rs` | Per the existing `Invariant` catalogue mechanical shape (`ALL`, `as_canonical`, `harness.rs` dispatch). Covers S-VM-87/88/89 (S-VM-24 already placed above) |
| `overdrive-init` guest agent | `overdrive-init` (NEW crate, `binary` class) | `src/main.rs` | `[D4]`; a new workspace member |

**Added 2026-08-11 (adversarial-review remediation, DWD-11) — crate/file placement for the thirteen new scenarios and the two contested-scenario rewrites:**

| Scenario scope | Crate | Path | Rationale |
|---|---|---|---|
| `MtlsInterceptWorker` gating, ungated-off arm (S-VM-74) | `overdrive-cli` | `tests/integration/vm_walking_skeleton.rs`, gated `integration-tests` | Same driving port and fixture family as S-VM-05, which it extends |
| `Vmm` real non-reflink substrate (S-VM-75) | `overdrive-cli` | `tests/integration/vm_walking_skeleton.rs`, gated `integration-tests` | Boot-time scenario, alongside S-VM-11…13 |
| `VmDriver::stop` totality against `SimVmm` (S-VM-76) | `overdrive-worker` | `tests/acceptance/vm_driver_stop_totality.rs` (NEW file), default lane | Component-scope, no Lima/real-CH needed — `SimVmm` only |
| Abandonment boundary / hydration read order / write-time terminality guard / P2-over-`VmReclamation` (S-VM-77…80) | `overdrive-core` | `tests/acceptance/vm_reclamation_plan_purity.rs` (EXISTING file, extended) or a new sibling `vm_reclamation_claim_lifecycle.rs` — DELIVER's own call once the `VmSupervision` sum type exists | In-memory, pure/component-scope, same family as S-VM-31/32/92 |
| Fourth evaluation — `svid_lifecycle` (S-VM-81) | `overdrive-cli` | `tests/integration/vm_reclamation_tier3.rs`, gated `integration-tests` | Real `overdrive serve` convergence loop, mTLS-composed |
| ESR invariant scenarios (S-VM-87, 88, 89) | `overdrive-sim` | `src/invariants/vm_reclamation.rs` | Same file as the pre-existing four-invariant placement above |
| `CgroupAccounting` adapter equivalence (S-VM-93) | `overdrive-host` | `tests/integration/cgroup_accounting_equivalence.rs`, gated `integration-tests` | Named exactly by ADR-0082 §D8, same shape as `vmm_equivalence.rs` / `vm_host_state_equivalence.rs` |
| Per-launch `FICLONE` self-application (S-VM-94) | `overdrive-host` | `tests/integration/vmm_ficlone_per_launch.rs` (NEW file), gated `integration-tests` | Real non-reflink fixture, adapter-level, not through the CLI |
| S-VM-35 (rewritten to the TOCTOU window) | `overdrive-cli` | `tests/integration/vm_walking_skeleton.rs`, gated `integration-tests` | Unchanged placement — the rewrite changes the scenario's content, not its file |
| S-VM-13 (narrowed to capability-flag classes) | `overdrive-cli` | `tests/integration/vm_walking_skeleton.rs`, gated `integration-tests` | Unchanged placement |

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

## Changelog

- 2026-08-11 — Initial DISTILL wave decisions captured. 0 contradictions in reconciliation (both the orchestrator's pre-verified summary and this session's independent full read agree). 74 scenarios across 9 user stories + 1 cross-cutting reconciler + 3 port-contract-enforcement scenarios, tagged and traced to all 10 KPIs. Walking skeleton: S-VM-01, one scenario, Slice 01. Adapter strategy: this project's four-tier model (Tier 1 in-memory default lane / Tier 3 real-Lima `integration-tests` lane), with `Sim*` fault injection at the port boundary for substrate-lie scenarios. Mandate 7 scaffolding: scoped to Slice 01 + three cross-cutting pure-function scenarios (15 scaffolds, verified compiling and RED by execution — `cargo check`, `cargo clippy -D warnings`, `cargo nextest run`, all clean); the remaining 59 scenarios' scaffolds are deferred to DELIVER's per-slice RED phase with exact file placement already committed in DWD-04. Two drafting corrections made and recorded (DWD-07): the no-subprocess CLI convention, and three dangling scenario references closed.
- 2026-08-11 — Peer review (Sentinel, `nw-acceptance-designer-reviewer`): NEEDS_REVISION (1 blocker, 2 high). Fixed, none waived (DWD-08): added the mandate-14 `@contract-shape:` tag to all 74 scenarios; corrected a 2-scenario undercount surfaced by the tagging pass (72 → 74, mechanically recounted) across `test-scenarios.md`, this file, and `feature-delta.md`'s DISTILL section; confirmed the deferred-scaffold forward-reference table (DWD-04) already satisfies the reviewer's other high finding.
- 2026-08-11 — Second-round adversarial review (Sentinel + Atlas, two independent fable dispatches, both `needs_revision`): FIXED (DWD-11). One BLOCKER (S-VM-88/89 phantom references + the third undefined §105a.11 invariant) — three scenarios defined under a new AC-20 (S-VM-87, 88, 89). Four systemic HIGH findings (the NEW-1 pins under-covered) — four scenarios under a new AC-19 (S-VM-77…80). Eight more HIGH findings — S-VM-81 (fourth evaluation), S-VM-93 (`CgroupAccounting` equivalence), S-VM-94 (per-launch `FICLONE`), S-VM-74 (`MtlsInterceptWorker` gating), S-VM-76 (`VmDriver::stop` totality, new AC-18, with a documented Driving Ports table carve-out), S-VM-13 narrowed + S-VM-75 added (non-reflink envelope-claim fix), S-VM-35 rewritten (TOCTOU, fixes the S-VM-12 contradiction), S-VM-49 reworded (fixes the S-VM-53 contradiction). Nine MEDIUM/LOW findings — S-VM-26/S-VM-20/S-VM-08/S-VM-44/S-VM-37 corrected in place; DWD-03/DWD-06 accounting errors fixed; a dst-lint-clause AC-ownership decision (DWD-09) and a kernel-matrix-ownership decision (DWD-10) recorded. One item SETTLED by explicit user ruling, not fixed: DWD-06a records that `.claude/rules/testing.md` governs over the generic skill's ADR-025 statement, so the scaffold deferral stands; ownership of per-slice scaffold authorship (crafter) and review (`nw-software-crafter-reviewer`) answered. Two items marked BLOCKED on the concurrent DESIGN pass, not guessed: S-VM-65's mid-run storage-daemon-death `TransitionReason` variant, and the `SimVmm`/`SimVmHostState` production-composition-root injection seam for S-VM-13/51/67. Scenario count 74 → 87; error/edge coverage 59% → 60%; zero dangling `S-VM-N` references (mechanically re-verified across all three artifacts).
- 2026-08-11 — Concurrent DESIGN pass ruled on both outstanding blockers (DWD-12). **RESOLVED**: S-VM-65's mid-run storage-daemon-death variant — ADR-0083 §D5 gained row 14 (`TransitionReason::VmStorageDaemonDied`), checked ahead of `ExitKind` entirely; S-VM-65 rewritten with a second scenario shape (guest self-reports `EXIT 0` after the daemon dies) that fails if the precedence ordering is wrong. **RESOLVED**: the `SimVmm` injection seam for S-VM-13/S-VM-51 — ADR-0083 §D8, `ServerConfig.vmm_override`, a whole-port substitution shaped after `mtls_identity_override`, not `dataplane_override` (rejected by name, §A10); both scenarios' crafter notes now name the seam and gating exactly. **STAYS BLOCKED, precisely**: S-VM-67 — ADR-0083 §D8 explicitly rules it outside the seam's reach (no `Vmm` method sits downstream of virtiofsd's sandbox check; no storage-daemon supervision port exists); its crafter note is corrected to state this is a scoping decision, not a missing seam name, and names the two candidate paths without choosing either. Upstream Issues reduced from two blocked items (four blocked scenario references) to one open item. Adapter Coverage Table's `Vmm (SimVmm)` row and Self-Review Checklist item 4 corrected to drop S-VM-67 (never covered by this seam). Scenario count unchanged at 87 (mechanically re-verified); no ADR, `brief.md`, or Rust file touched.
- 2026-08-11 — User ruling closes the last open item (DWD-13). **RESOLVED**: S-VM-67 — path (b) chosen: `[D8d]`'s `--sandbox=namespace`-unavailable case is verified at the launch-argument construction layer (private fields, one rendering site, a pure unit test on the rendered value — the same enforcement tier ADR-0082 §D2.1 already uses for `image_type=raw`), never through a real `overdrive serve`. **This feature mints no storage-daemon supervision port.** S-VM-67 rewritten in full: `@tier3`/`@real-io` → `@tier1`/`@in-memory`, `@contract-shape:bounded-change` → `@contract-shape:pure-function`, `@property` gained (mirrors S-VM-17's pure-function-plus-`@error_path` precedent, `@error_path` retained), driving port changed from `overdrive deploy` to the storage daemon's launch-argument rendering site (a not-yet-ADR-pinned Slice 04 type — DELIVER's own naming, per CLAUDE.md § "Implement to the design"). The scenario's `Then` now carries an explicit boundary statement: it proves only what argument the rendering function constructs, never that a running `virtiofsd` enforces it or that the platform genuinely fails closed end-to-end — both stay an undischarged Tier-3 property of Slice 04. No separate Tier-3 runtime-half scenario was added (reasoned in DWD-13: no port to inject through, no genuinely-lying host in the one-kernel Lima envelope, and minting either now would invent API surface past the design). Sibling references corrected: the `@real-io` Adapter Coverage Table's virtiofsd row, the US-VM-9 AC-to-Scenario Traceability row, Self-Review Checklist item 4 (all three previously touched by DWD-12 for the S-VM-13/S-VM-51 resolution, now re-verified against S-VM-67's new resolution), plus two references DWD-12 did not reach: the top-of-file Driving Ports table's `overdrive deploy` row (range corrected to exclude S-VM-67; a new row added for the pure-function driving port) and this file's own DWD-04 crate-placement table (same range correction). Error/Edge Path Coverage counts updated: `@property` 20 → 21, `@tier3`/`@real-io` 61 → 60, `@tier1`/`@in-memory` 29 → 30; `@error_path` unchanged at 40; total unchanged at 87; error+edge coverage unchanged at 60%. Upstream Issues now shows **zero** open items; all three resolved items (S-VM-65, the S-VM-13/S-VM-51 seam, S-VM-67) kept struck-through for the audit trail. No ADR, `brief.md`, or Rust file touched by this DISTILL pass (the ADR amendments already landed via the concurrent DESIGN pass before this pass started). No GitHub issue created; #259–#263 remain the only real numbers in scope, #264 closed. No commit made.
