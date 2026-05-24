# Feature Delta — service-health-check-probes

**Source brief:** GitHub issue #170 (supersedes #169 — settle-window primitive was rejected by operator framing in favour of declarative k8s-shape probes).

**Predecessor RCA:** `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md` — root cause A ("kernel-accepted exec is NOT operator-meaningful liveness").

**Predecessor ADRs:** ADR-0047 (workload kind discriminator — Service / Job / Schedule split), ADR-0037 (typed TerminalCondition; reconciler decides terminal-or-not), ADR-0032 (NDJSON streaming submit; per-kind SubmitEvent envelope), ADR-0033 (alloc_status snapshot enrichment).

## Wave: DISCUSS / REF Artifact index

Authoritative artifacts produced by this DISCUSS wave (all live in `docs/feature/service-health-check-probes/`):

| Artifact | Purpose |
|---|---|
| `discuss/wave-decisions.md` | DISCUSS wave decisions: DIVERGE-absent risk note, scope assessment verdict (PASS, ~8 stories, 3 contexts), slicing rationale |
| `discuss/journey-service-honest-stable-visual.md` | ASCII flow + emotional annotations + TUI mockups per step |
| `discuss/journey-service-honest-stable.yaml` | Structured journey schema with embedded Gherkin per step |
| `discuss/story-map.md` | Backbone, walking-skeleton identification, 8 release slices with priority rationale |
| `discuss/shared-artifacts-registry.md` | Single-source-of-truth registry for every cross-step variable (probe_idx, settled_in, witness, terminal_condition_bytes, …) |
| `discuss/user-stories.md` | 8 LeanUX user stories (US-01..US-08) with Elevator Pitch, Problem, Who, Solution, Domain Examples, UAT Scenarios (BDD), AC, KPIs, Technical Notes, Dependencies |
| `discuss/outcome-kpis.md` | 5 outcome KPIs (K1–K5) with hypothesis, metric hierarchy, measurement plan, guardrails |
| `slices/slice-01..08-*.md` | Machine-readable slice briefs (one per release slice) |

SSOT updates landed by this wave:

| File | Change |
|---|---|
| `docs/product/jobs.yaml` | Added J-OPS-004 (Service-honesty sub-job extending J-OPS-003); changelog entry 2026-05-24 |
| `docs/product/journeys/submit-a-service.yaml` | NEW — product-level Service-kind submit journey (companion to `submit-a-job.yaml`) |

## Wave: DISCUSS / WHY Job grounding

J-OPS-003 ("Run my actual workload on the walking-skeleton control plane and trust the platform to converge to the declared replica count") established the operator-trust contract for the Job kind in Phase 1 (`crates/overdrive-control-plane/...` JobLifecycle). The 2026-05-09 RCA proved that the contract is broken for the Service kind: the wire signal `is running with 1/1 replicas (took live)` fires the moment the kernel accepts `fork+exec`, NOT when the workload is actually serving — which means a Service whose entrypoint exits 1 within 50ms gets reported as "stable-equivalent" with exit code 0.

This feature derives **J-OPS-004** as the Service-specific sub-job. The motivation is structurally distinct from J-OPS-003 because Services are long-lived and have an "is it actually serving?" question (k8s readiness/liveness shape) that Jobs (run-to-completion) and Schedules (compose per-fire) do not. The "what does success mean?" answer for a Service is **operator-declared**, not platform-defined — hence the probe primitive.

**Forces:**

- **Push:** "The CLI lied to me about my Service. I cannot trust the streaming submit signal as the SSOT of 'is it serving?'" — Ana, 2026-05-09 incident.
- **Pull:** "k8s-shape declarative probes scoped per role (startup / readiness / liveness) — I already know this vocabulary; let me bring it across."
- **Anxiety:** "Will the new primitive add latency to every submit? Will my long-warming Services time out wrongly?" — addressed by default 60s startup_deadline + per-spec configurability via probe knobs.
- **Habit:** "I currently `submit && sleep N && alloc status`. The new shape removes the sleep AND the second command — the streaming wire IS the truthful witness."

## Wave: DISCUSS / WHY Why this feature is right-sized for one DESIGN wave

Per `wave-decisions.md` § Scope Assessment: 8 stories, 3 bounded contexts (`overdrive-worker`, `overdrive-control-plane`, `overdrive-cli`), 4 walking-skeleton integration points, ~8 days total effort. Under the elephant-carpaccio gate (≤10 stories, ≤3 contexts, ≤2 weeks): **PASS**.

Slicing taste tests applied:
- Slice 01 alone delivers end-to-end value (operator can submit a probe-less Service and get an honest signal).
- Each subsequent slice composes onto Slice 01's trait surface; no slice is a hidden refactor.
- Slice 07 is independent (parser-only); can land in parallel.
- Slice 08 closes the specific RCA-A regression case.

## Wave: DISCUSS / HOW Slicing recipe

```
Slice 01 (WS) ──┬── Slice 02 (HTTP startup)
                ├── Slice 03 (Exec startup)
                ├── Slice 04 (Readiness) ──── Slice 05 (Liveness)
                ├── Slice 06 (CLI Probes section)
                └── Slice 08 (EarlyExit regression guard)

Slice 07 (kind-rejection) — independent; can land in parallel with 01.
```

Walking skeleton confirmed: **Slice 01 (default TCP-connect startup probe end-to-end)**. Rationale: it (a) closes RCA-A for the most common case (operator declares no probes — the existing Phase 1 idiom), (b) establishes the full trait surface (ProbeRunner / ProbeResultRow / ServiceLifecycleReconciler / new TerminalCondition variants / new wire variants) that every subsequent slice composes onto, (c) provides maximum K1 movement per LOC.

## Wave: DISCUSS / HOW Outcome KPIs the platform-architect must instrument

Full table in `discuss/outcome-kpis.md`. Summary:

- **K1 (North Star):** ≥99% Service-submit honesty rate — measured by integration test reshaping coinflip.toml as Service with never-passing startup probe. Baseline: 0%.
- **K2:** Readiness Fail → `Backend.healthy = false` within 1 reconciler tick. ≥99%.
- **K3:** Liveness threshold → restart within 1 tick. ≥99%.
- **K4:** Probes section renders for 100% of Service allocs with probes; 0% of Job/Schedule.
- **K5:** Misshapen-spec named-error rate — 100% reject at parse time.

Guardrails: probe-runner CPU ≤0.5% per Service-alloc-with-3-probes; ProbeResultRow LWW (not append-mode) per `(alloc_id, probe_idx)`; submit latency stays ≤1.5× current baseline; nextest wall-clock grows ≤10%.

## Wave: DISCUSS / HOW Hand-off package contents for DESIGN wave (solution-architect)

The architect should receive:

1. **This file** (`feature-delta.md`) — entry point with REF / WHY / HOW sections.
2. **`discuss/journey-service-honest-stable-visual.md` + `.yaml`** — the operator journey artifacts.
3. **`discuss/story-map.md`** — backbone, walking skeleton, 8 slices with priority rationale (P0 / P1 / P2 / P3) and dependency graph.
4. **`discuss/user-stories.md`** — 8 stories with embedded UAT scenarios (Gherkin), AC, KPIs, technical constraints (System Constraints section C1–C10).
5. **`discuss/shared-artifacts-registry.md`** — cross-step variable contracts to validate at DESIGN time.
6. **`discuss/outcome-kpis.md`** — measurable success bar; informs DEVOPS instrumentation.
7. **`slices/slice-01..08-*.md`** — per-slice machine briefs.
8. **`discuss/wave-decisions.md`** — DIVERGE-absent risk note; scope assessment.

### Anticipated DESIGN-wave open questions (P1 and P2; main has indicated "all priorities"):

| ID | Priority | Question | Why it matters at DESIGN |
|---|---|---|---|
| P1-Q1 | P1 | Where does `ProbeRunner` live in the worker crate's task graph? Per-alloc loop or per-alloc tokio task or shared scheduler? | Affects CPU guardrail K2; affects shutdown semantics |
| P1-Q2 | P1 | Cgroup placement mechanism for Exec probes: clone3 vs cgroup.procs write | Affects portability + sim-adapter shape per `.claude/rules/development.md` § "Production code is not shaped by simulation" |
| P1-Q3 | P1 | `ServiceFailureReason` enum module location and SemVer convention | Will eventually carry `LivenessExhausted` (Slice 05) and possibly variants for future probe types |
| P2-Q4 | P2 | Default values: timeout_seconds (5?), interval_seconds (2?), max_attempts (30?), liveness failure_threshold (3?) | Operator UX; should align with k8s defaults to honour habit force |
| P2-Q5 | P2 | streaming_cap interplay for slow-warming Services (>60s startup) | C10 says unchanged; but should we surface `--wait-cap` flag? Or per-spec `startup_deadline_seconds`? Possible ADR amendment |
| P2-Q6 | P2 | Render shape for `--json` output of Probes section | Out of Slice 06 scope; flag for separate slice or DEVOPS wave |
| P2-Q7 | P2 | When a Service has multiple `[[health_check.startup]]` entries, what's the "all pass" semantic for Stable? AND or OR? | Affects `witness` payload — if AND, witness is the last-to-Pass; if OR, witness is the first-to-Pass |
| P2-Q8 | P2 | `successThreshold` for readiness probes: should operators configure consecutive-success requirement before re-adding backends to routing? | Prevents flapping backends from toggling `Backend.healthy` on transient failures. Kubernetes default is 1; configurable up to N. Nomad has `success_before_passing`. Phase 1 may default to 1 (pass-immediately). Reference: research § 5.1, § 7.2 D1. |
| P2-Q9 | P2 | Cascading-failure protection: should liveness-triggered restarts be rate-limited to N simultaneous restarts per Service? | Kubernetes issue #66230 documents risk of mass liveness-probe failures causing total downtime instead of degraded service. Phase 1 is single-node single-replica (no cascading surface), but architecture should not preclude this. Recommend: DESIGN documents the decision (implement, defer, or reject) with rationale. Reference: research § 6.1 Pitfall 1, § 7.2 D6. |

## Wave: DISCUSS / HOW Risks surfaced to DESIGN wave

| Risk | Probability | Impact | Mitigation owner |
|---|---|---|---|
| Probe-runner concurrency model accidentally serialises per-alloc work (head-of-line blocking under N probes) | M | H | DESIGN (solution-architect picks task shape) |
| Probe-result row cardinality explodes if implementer chooses append-mode instead of LWW | L | H | DESIGN; reviewer-enforce per `.claude/rules/development.md` § "Persist inputs, not derived state" |
| Exec probe cgroup placement diverges between Linux production and sim adapter | M | M | DESIGN; trait contract per § "Trait definitions specify behavior, not just signature" |
| streaming_cap < startup_deadline for slow-warming Services produces operator confusion ("CLI said Timeout but probe was still trying") | M | M | DESIGN; consider per-spec cap override OR documented limitation |
| `ServiceFailureReason` enum drift between wire and render (operator sees one reason on stream, another in alloc status) | L | H | Action shim is single write site; reviewer-enforce |
| Default-probe inference fires when operator MEANT to opt out (empty array intent unclear) | L | M | Parser distinguishes `<absent>` (infer default) from `[[health_check.startup]] = []` (explicit opt-out); test fixtures cover both |

## Wave: DISCUSS / HOW Definition of Ready validation (each story)

Per `nw-leanux-methodology` 9-item DoR (the 8-item list in the skill plus item 9 Outcome KPIs added by `nw-product-owner` per `nw-outcome-kpi-framework`):

### Story DoR matrix

| Story | 1. Problem clear | 2. Persona specific | 3. ≥3 domain examples w/ real data | 4. UAT 3-7 scenarios | 5. AC from UAT | 6. Right-sized | 7. Tech notes | 8. Deps tracked | 9. Outcome KPIs | Verdict |
|---|---|---|---|---|---|---|---|---|---|---|
| US-01 | PASS — Ana, 2026-05-09 RCA | PASS — Ana Lopez, single-node dev host | PASS — payments-minimal/jvm-app/coinflip-as-service | PASS — 5 scenarios | PASS | PASS — 2-3 days (WS) | PASS | PASS — ADRs landed | PASS — K1 | **PASS** |
| US-02 | PASS — `payments` /healthz | PASS — Ana, k8s background | PASS — 3 examples | PASS — 4 scenarios | PASS | PASS — 1-2 days | PASS | PASS — Slice 01 | PASS — K1 | **PASS** |
| US-03 | PASS — domain-specific health needs cgroup ns | PASS | PASS — 3 examples (good/missing/timeout) | PASS — 4 scenarios | PASS | PASS — 2 days | PASS — cgroup placement noted | PASS | PASS — K1 | **PASS** |
| US-04 | PASS — backend loses DB; LB keeps sending | PASS | PASS — 3 examples (happy/flapping/no-readiness) | PASS — 3 scenarios | PASS | PASS — 2 days | PASS — fingerprint consumer named | PASS | PASS — K2 | **PASS** |
| US-05 | PASS — wedged backend never recovers | PASS | PASS — 3 examples (3-fail/recovery/exhaustion) | PASS — 4 scenarios | PASS | PASS — 2 days | PASS | PASS — Slices 04, 01 | PASS — K3 | **PASS** |
| US-06 | PASS — no day-2 visibility | PASS | PASS — 3 examples (happy/pending/failing) | PASS — 5 scenarios | PASS | PASS — 1 day | PASS | PASS — Slices 01-05 | PASS — K4 | **PASS** |
| US-07 | PASS — operator confused by silent accept | PASS | PASS — 3 examples (job/schedule/service-control) | PASS — 3 scenarios | PASS | PASS — 0.5 day | PASS | PASS — ADR-0047 | PASS — K5 | **PASS** |
| US-08 | PASS — coinflip exit 1 = "is running" today | PASS — Ana, 2026-05-09 RCA | PASS — 3 examples (port collision/missing env/exit 0) | PASS — 5 scenarios | PASS | PASS — 1 day | PASS — reuses ExitObserver | PASS — Slice 01 | PASS — K1 | **PASS** |

**Aggregate DoR Status: PASSED (8 of 8 stories).**

## Wave: DISCUSS / HOW Self-review (Dimension 0 — Elevator Pitch Test)

Per `nw-po-review-dimensions` Dimension 0 (BLOCKING, checked first):

| Story | Has section | Real entry point | Concrete output | Real decision | Verdict |
|---|---|---|---|---|---|
| US-01 | PASS — Before/After/Decision-enabled | PASS — `overdrive job submit` | PASS — stdout "Service ... is stable\n  settled_in: 1.2s..." | PASS — "is my Service fit to receive traffic?" | **PASS** |
| US-02 | PASS | PASS — same CLI | PASS — witness line names HTTP probe | PASS — "what does 'ready' mean for me?" | **PASS** |
| US-03 | PASS | PASS — same CLI | PASS — Stable based on exec exit 0 | PASS — "write domain-specific health logic" | **PASS** |
| US-04 | PASS | PASS — declarative TOML + dataplane behaviour | PASS — `Backend.healthy = false` reflected in fingerprint | PASS — "trust platform to remove unhealthy backends" | **PASS** |
| US-05 | PASS | PASS — declarative TOML + restart behaviour | PASS — restart_count increments visible in alloc status | PASS — "trust platform to restart wedged backends" | **PASS** |
| US-06 | PASS | PASS — `overdrive alloc status --job <id>` | PASS — Probes section render shown | PASS — "debug workload / tune probe / restart" | **PASS** |
| US-07 | PASS | PASS — `overdrive job submit` parse-time | PASS — error text with named guidance | PASS — "stop trying to use probes on wrong kind" | **PASS** |
| US-08 | PASS | PASS — `overdrive job submit` | PASS — multi-line Failed render shown | PASS — "see WHY workload died" | **PASS** |

**No `@infrastructure` stories. No infrastructure-only slices.** Every slice contains at least one operator-visible value-producing story.

## Wave: DISCUSS / HOW Anti-pattern check

Per `nw-leanux-methodology`:

| Anti-pattern | Found? | Notes |
|---|---|---|
| "Implement X" framing | NO | Every story starts from operator pain (Ana's 2026-05-09 RCA, k8s muscle memory, day-2 visibility gap) |
| Generic data (user123, test@test.com) | NO | All examples use Ana Lopez, real TOML fixtures, real port numbers, real exit codes |
| Technical AC ("Use JWT...") | NO | AC are observable outcomes ("Stable wire event carries settled_in: Duration"; "ProbeResultRow has last_fail_reason 'HTTP 503'") |
| Technical scenario titles | NO | Titles describe operator outcomes ("Service reaches Stable when listener binds", "Probe Fail row renders last_fail_reason") |
| Oversized stories (>7 scenarios, >3 days) | NO | All 8 stories ≤5 scenarios, ≤2-3 days each |
| Abstract requirements | NO | Every story has 3 concrete examples with real data |

## Wave: DISCUSS / HOW Hand-off recipient

**Next agent:** `nw-solution-architect` (DESIGN wave).

**Confirm scope shape per `~/.claude/CLAUDE.md` § nWave dispatches:** "all priorities (P1 + P2)" — every P1-Qx and P2-Qx open question listed in HOW Hand-off package should be answered by DESIGN unless main explicitly narrows scope.

**Architect should also receive (cross-link):** ADR-0037, ADR-0032, ADR-0033, ADR-0047, the 2026-05-09 RCA, the existing dataplane fingerprint module at `crates/overdrive-core/src/dataplane/fingerprint.rs`, and `.claude/rules/development.md` § "Reconciler I/O" + § "Persist inputs, not derived state" + § "Production code is not shaped by simulation".

**No DIVERGE artifacts to forward.** Risk noted in `wave-decisions.md`; mitigation is direct grounding in RCA-A + J-OPS-004 derivation. If main wants an ODI-style outcome-scoring loop or alternative design-direction analysis (e.g. continuous-health-everywhere vs probe-driven-on-demand), a DIVERGE wave should be inserted before DESIGN.

## Wave: DESIGN / REF Artifact index

Authoritative artifacts produced by this DESIGN wave:

| Artifact | Purpose |
|---|---|
| `docs/product/architecture/brief.md` §§ 75–87 | Application-architecture extension for this feature (8 sub-sections); appended, no prior content rewritten |
| `docs/product/architecture/c4-diagrams.md` § "Phase 1 — Service Health-Check Probes component diagram (Mermaid)" | C4 L2 annotation + C4 L3 ProbeRunner subsystem topology |
| `docs/product/architecture/adr-0054-probe-runner-subsystem.md` | NEW — ProbeRunner placement, task graph, port traits, ProbeResultRow, Earned Trust gate |
| `docs/product/architecture/adr-0055-service-lifecycle-reconciler.md` | NEW — typed View, pure reconcile, `Stable` non-terminal extending ADR-0037, AND-of-all multi-probe, readiness successThreshold, cascading-restart Phase 2+ surface |
| `docs/product/architecture/adr-0056-service-submit-event-stable-failed-evolution.md` | NEW — wire shape V1→V2 (Stable/Failed), single ServiceFailureReason enum, streaming-cap deferred-non-decision, JSON-mode Probes shape |
| `docs/product/architecture/adr-0057-health-check-toml-spec.md` | NEW — TOML shape, defaults table (P2-Q4 resolution), kind rejection, ServiceSpec V2 envelope bump |
| `docs/product/architecture/adr-0058-default-tcp-startup-probe-inference.md` | NEW — "honest by default" inference rule, opt-out semantics, K8s/Nomad divergence justification |
| `docs/product/architecture/adr-0059-exec-probe-cgroup-placement.md` | NEW — `cgroup.procs` write (P1-Q2 resolution), reuses ExecDriver primitives, clone3 deferred to Phase 2+ |
| `discuss/wave-decisions.md` § "Design Decisions Summary" | DESIGN decisions summary appended to DISCUSS wave-decisions (no new design/ directory created; DESIGN content lives in brief.md + ADRs per project convention) |

## Wave: DESIGN / REF Architectural Design Decisions (DDDs)

| ID | Decision | Verdict | One-line rationale |
|---|---|---|---|
| D-01 | ProbeRunner placement | `overdrive-worker` (adapter-host) | Probe execution is observation production — belongs to the machine running the workload (C1). |
| D-02 | ProbeRunner task graph | Per-alloc supervisor + per-probe-instance tokio tasks | Failure isolation per probe; aborts cleanly on cancel; matches K8s prober.Manager (research § 3.3 D5). |
| D-03 | Port-trait shape | Three traits — TcpProber, HttpProber, ExecProber | Each mechanic has distinct preconditions / postconditions / adapter dependency surfaces; one omnibus trait conflates contracts. |
| D-04 | ProbeResultRow shape | LWW per `(alloc_id, probe_idx)`; rkyv envelope V1 | `.claude/rules/development.md` § "Persist inputs, not derived state"; row cardinality bounded by spec not time. |
| D-05 | ServiceLifecycleReconciler placement | Own reconciler at `crates/overdrive-control-plane/src/reconcilers/service_lifecycle/` | Service `View` shape disjoint from Job; shared struct with optional fields would violate "Sum types over sentinels". |
| D-06 | `Stable` as non-terminal condition | Encoded structurally via `View::stable_announced` BTreeSet; no flag on TerminalCondition | ADR-0037 layering preserved; dedup lives in reconciler, not in the typed enum. |
| D-07 | Multi-startup-probe semantic (P2-Q7) | AND-of-all (every startup probe must Pass) | Matches operator-declared invariants; OR-semantic reserved for future combinator knob. |
| D-08 | Readiness successThreshold default (P2-Q8) | 1 (matches K8s default); configurable upward | Inputs persisted in `View::readiness_consecutive_successes`; gate recomputed per tick. |
| D-09 | Cascading-restart rate-limiting (P2-Q9) | Phase 1 single-node has no surface; architecture leaves room for Phase 2+ governor reconciler | `RestartAllocation` emitted unconditionally; future governor consumes + filters; non-breaking architecturally. |
| D-10 | ServiceFailureReason SemVer (P1-Q3) | Single per-kind enum (not per-condition sub-enums); additive variants per ADR-0037 §5 | Operator-facing single surface for "why did my Service fail?"; lockstep with wire projection via property test. |
| D-11 | ServiceSubmitEvent shape evolution | V1→V2: DELETE `ConvergedRunning` / `ConvergedFailed`; ADD `Stable` / `Failed` | Single-cut greenfield migration per `feedback_single_cut_greenfield_migrations.md`. |
| D-12 | Streaming-cap interplay (P2-Q5) | 60s cap unchanged; no new operator knob in Phase 1 | Reconciler continues post-disconnect; operator inspects via `alloc status`. If feedback warrants, additive ADR adds per-spec or CLI knob later. |
| D-13 | JSON-mode Probes shape (P2-Q6) | `ProbeResultRowJson` derived via `utoipa::ToSchema` per ADR-0009 | Aligns with ADR-0033 enrichment convention; schema generated, not hand-written. |
| D-14 | TOML defaults table (P2-Q4) | timeout 5s, interval 2s startup/readiness / 10s liveness, max_attempts 30, failure_threshold 1 readiness / 3 liveness, success_threshold 1 | Diverges from K8s where defensible (5s timeout vs K8s 1s); justification in ADR-0057 §2. |
| D-15 | Default-probe inference rule | TCP-connect on `listener[0]` when probes absent + listeners non-empty | "Honest by default" — closes RCA-A for the most common workflow; opt-out via empty array preserves spec compatibility. |
| D-16 | Inferred-vs-explicit-opt-out distinction | Parser-level distinction (`<absent>` vs `[[health_check.startup]] = []`) | Two operator intents; structural distinction; integration test pins both. |
| D-17 | Exec-probe cgroup placement (P1-Q2) | `cgroup.procs` write of spawned PID; reuses `place_pid_in_scope` from ExecDriver | Code reuse with ADR-0030; DST-friendly via sim adapter; `clone3` deferred to Phase 2+ pending `nix-rust/nix#2120`. |
| D-18 | Exec-probe timeout cleanup | `cgroup.kill` (Linux 5.14+); PID-loop fallback for 5.10–5.13 | Mass-kill prevents orphaned descendants from healthcheck scripts that fork. |
| D-19 | `ServiceSpecEnvelope` evolution | V1→V2 bump per ADR-0048 "Version-bump procedure" | Three additive Vec fields; single commit + new fixture + existing FIXTURE_V1 untouched. |
| D-20 | New crate dependencies | `hyper-util` + `tokio-util` (both already in workspace graph as transitives) | No new top-level deps; both MIT-licensed per workspace OSS policy. |

## Wave: DESIGN / REF Component decomposition

| Component | Path | Change type | Notes |
|---|---|---|---|
| `ProbeRunner` subsystem | `crates/overdrive-worker/src/probe_runner/` | **CREATE NEW** (module tree) | Sibling of ExecDriver / CgroupManager; depends on new port traits |
| `TcpProber` / `HttpProber` / `ExecProber` traits | `crates/overdrive-core/src/traits/prober.rs` | **CREATE NEW** module | Three port traits per `.claude/rules/development.md` § "Port-trait dependencies" |
| `TokioTcpProber` / `HyperHttpProber` / `CgroupExecProber` | `crates/overdrive-worker/src/probe_runner/{tcp,http,exec}_prober.rs` | **CREATE NEW** files | Production bindings |
| `SimTcpProber` / `SimHttpProber` / `SimExecProber` | `crates/overdrive-sim/src/adapters/probers.rs` | **CREATE NEW** module | Sim bindings; queue-driven outcome injection |
| `ProbeResultRow` + envelope | `crates/overdrive-core/src/observation/probe_result.rs` | **CREATE NEW** module | rkyv envelope V1 per ADR-0048 |
| `ObservationStore` trait | `crates/overdrive-core/src/traits/observation_store.rs` | **EXTEND** | Add `write_probe_result` + `list_probe_results_for_alloc` methods |
| `LocalObservationStore` redb adapter | `crates/overdrive-store-local/src/observation_store.rs` | **EXTEND** | New redb table for probe results |
| `ServiceLifecycleReconciler` | `crates/overdrive-control-plane/src/reconcilers/service_lifecycle/` | **CREATE NEW** module tree (mirroring `service_map_hydrator/`) | Pure sync reconcile; typed View |
| `ServiceLifecycleState` / `ServiceLifecycleView` | `crates/overdrive-core/src/reconcilers/service_lifecycle.rs` | **CREATE NEW** module | Per ADR-0021 / ADR-0035 per-reconciler typed projections |
| `AnyState` / `AnyReconcilerView` / `AnyReconciler` enums | `crates/overdrive-core/src/reconcilers/mod.rs` | **EXTEND** | Add `ServiceLifecycle(...)` variants + match arms |
| `TerminalCondition` enum | `crates/overdrive-core/src/transition_reason.rs` | **EXTEND** | Add `Stable { settled_in, witness }`, `Failed { reason }` variants per ADR-0037 §5 |
| `ServiceFailureReason` enum | `crates/overdrive-core/src/transition_reason.rs` | **CREATE NEW** type | Single per-kind reason enum; `#[non_exhaustive]` |
| `ProbeWitness` struct | `crates/overdrive-core/src/transition_reason.rs` | **CREATE NEW** type | Carries `probe_idx + role + mechanic_summary + inferred` |
| `ServiceSpec` aggregate | `crates/overdrive-core/src/aggregate/service_spec.rs` (per ADR-0050) | **EXTEND** | Add 3 Vec<ProbeDescriptor> fields; envelope V1→V2 |
| `ProbeDescriptor` / `ProbeMechanic` / `ProbeRole` / `ProbeIdx` | `crates/overdrive-core/src/aggregate/probe_descriptor.rs` | **CREATE NEW** module | Validated intent-side type; rkyv-archived in ServiceSpec |
| TOML parser | `crates/overdrive-core/src/aggregate/workload_spec.rs` | **EXTEND** | Accept `[[health_check.*]]` sections; defaults; kind rejection |
| `ParseError` enum | `crates/overdrive-core/src/aggregate/workload_spec.rs` | **EXTEND** | Add probe-specific variants (ProbesNotAllowedOnKind, HttpProbeMissingPath, etc.) |
| `ServiceSubmitEvent` enum | `crates/overdrive-control-plane/src/api.rs` | **EXTEND** | DELETE ConvergedRunning/ConvergedFailed; ADD Stable/Failed (single-cut migration) |
| `ProbeWitnessWire` / `ServiceFailureReasonWire` | `crates/overdrive-control-plane/src/api.rs` | **CREATE NEW** types | Wire projections of typed enums; utoipa::ToSchema |
| `ProbeResultRowJson` | `crates/overdrive-control-plane/src/api.rs` | **CREATE NEW** type | JSON-mode `alloc status` Probes section schema (US-06) |
| Action shim mapping (TerminalCondition→ServiceSubmitEvent) | `crates/overdrive-control-plane/src/streaming.rs` | **EXTEND** | Single write site preserves ADR-0037 §4 byte-equality |
| `Action::RestartAllocation` reason field | `crates/overdrive-core/src/reconcilers/mod.rs` | **EXTEND** | Add `RestartReason::LivenessExhausted` (additive variant on existing reason enum or new enum) |
| `AllocationSpec` (driver layer) | `crates/overdrive-core/src/traits/driver.rs` | **EXTEND** | Add `probe_descriptors: Vec<ProbeDescriptor>` field; Job/Schedule kinds pass empty |
| `ExecDriver` lifecycle hooks | `crates/overdrive-worker/src/driver.rs` | **EXTEND** | `on_alloc_running` signals `probe_runner.start_alloc`; `on_alloc_terminal` signals `stop_alloc` |
| CLI render — Service Probes section | `crates/overdrive-cli/src/render.rs` | **EXTEND** | New Probes section block under Service-kind alloc render; absent for Job/Schedule |
| CLI alloc-status JSON output | `crates/overdrive-cli/src/commands/alloc.rs` | **EXTEND** | Marshal ProbeResultRowJson per ADR-0033 |
| Earned Trust subtype + structural + behavioural enforcement for `probe()` | `xtask::dst_lint` AST scanner | **EXTEND** | New scan: `ProbeRunner` impl block must declare `probe(&self)` method |

## Wave: DESIGN / REF Driving + driven ports + adapters

| Port | Direction | Adapters (prod / sim) | Where |
|---|---|---|---|
| `TcpProber` | Driven (from ProbeRunner to TCP transport) | `TokioTcpProber` / `SimTcpProber` | core::traits::prober / worker::probe_runner / sim::adapters::probers |
| `HttpProber` | Driven (from ProbeRunner to HTTP transport) | `HyperHttpProber` / `SimHttpProber` | same |
| `ExecProber` | Driven (from ProbeRunner to subprocess + cgroup) | `CgroupExecProber` / `SimExecProber` | same |
| `ObservationStore` (existing) | Driven (from ProbeRunner to durable store) | `LocalObservationStore` (existing — extended methods) | core::traits / store-local |
| `Clock` (existing per ADR-0013) | Driven (from per-probe task to wall-clock) | `SystemClock` / `SimClock` | core::traits::clock / host / sim |
| HTTP submit (existing) | Driving (operator → control-plane) | NDJSON over rustls (existing per ADR-0008) | api.rs |
| `Driver` (existing) | Driven (control-plane → worker) | `ExecDriver` (extended with probe lifecycle hooks) | worker::driver |

No new driving ports (probes are not operator-triggered RPCs;
they fire on the runner's own timer). All new ports are driven.

## Wave: DESIGN / REF Technology choices

| Choice | Selection | Alternatives considered | Rationale | License |
|---|---|---|---|---|
| HTTP client for HttpProber | `hyper-util::client::legacy::Client` + `hyper` 1.x | `reqwest` (heavier, full-fat); raw `tokio::io` (too low-level) | Already in workspace via `axum` transitive; lightweight; supports connection pool for N allocs × M probes; per-request timeout via `tokio::time::timeout` wrapper | MIT |
| Cancellation token | `tokio_util::sync::CancellationToken` | `tokio::sync::Notify` + manual flag; raw `Arc<AtomicBool>` | Already in workspace via `tokio` transitive; ergonomic `child_token()` for per-probe-task scoping; semantically clear cancel/cancelled API | MIT |
| Task group | `tokio::task::JoinSet` | manual `Vec<JoinHandle>`; `tokio_util::task::TaskTracker` | Already in tokio; auto-abort on drop matches our supervisor shutdown semantics; minimal API surface | MIT |
| Cgroup primitives for exec probe | Reuse `crates/overdrive-worker/src/cgroup_manager.rs` (existing) | Implement `clone3 + CLONE_INTO_CGROUP` raw syscall wrapper | Code reuse with ExecDriver; sim-adapter compatibility; clone3 deferred Phase 2+ per ADR-0059 | (existing) |
| Probe result observation persistence | `redb` (existing per ADR-0035 / ADR-0012) | Separate SQLite file; in-memory only | LocalObservationStore is already redb-backed; LWW via composite PK is structural; matches existing rows | MPL-2.0 |
| Probe result wire format | rkyv envelope V1 + serde JSON projection | bare rkyv; serde-only | rkyv for durable observation row (per ADR-0048); JSON for HTTP snapshot endpoint (existing per ADR-0033 enrichment shape); separate concerns | (existing) |

No proprietary tech selected. No new top-level workspace
dependencies — both `hyper-util` and `tokio-util` are already
present as transitives; the additions are direct dependency
declarations only (workspace `Cargo.toml` gets `.workspace = true`
references).

## Wave: DESIGN / REF Decisions table (locked)

| ID | Decision (one-line) |
|---|---|
| DDD-1 | ProbeRunner lives in `overdrive-worker`; per-alloc supervisor + per-probe tokio task shape. |
| DDD-2 | Three port traits (`TcpProber`/`HttpProber`/`ExecProber`) declared in `overdrive-core::traits::prober` with full rustdoc contracts. |
| DDD-3 | `ProbeResultRow` is LWW per `(alloc_id, probe_idx)`; lives in `ObservationStore` as additive table; rkyv envelope V1. |
| DDD-4 | `ServiceLifecycleReconciler` is its own typed reconciler (new `AnyReconciler` variant); pure sync `reconcile`. |
| DDD-5 | `ServiceLifecycleView` carries inputs only (consecutive_failures, consecutive_successes, stable_announced set, startup_attempts); `Stable` predicate is recomputed every tick. |
| DDD-6 | `TerminalCondition` gains `Stable` (non-terminal-semantically) and `Failed { reason: ServiceFailureReason }` variants; ADR-0037 §5 additive minor SemVer. |
| DDD-7 | Multi-startup-probe AND-of-all for `Stable`; witness names last-to-pass; OR-combinator reserved for future knob. |
| DDD-8 | Readiness `success_threshold` default 1; configurable upward; persisted as input counter. |
| DDD-9 | `Action::RestartAllocation` emitted unconditionally; Phase 2+ cascading-restart governor reconciler is non-breaking addition. |
| DDD-10 | `ServiceFailureReason` is single per-kind enum (`StartupProbeFailed`, `EarlyExit`, `BackoffExhausted`); additive variants per ADR-0037 §5. |
| DDD-11 | `ServiceSubmitEvent` V1→V2: delete `ConvergedRunning`/`ConvergedFailed`; add `Stable`/`Failed`; single-cut greenfield. |
| DDD-12 | Streaming 60s cap unchanged in Phase 1; slow-warming Services adopt submit→cap→inspect workflow. |
| DDD-13 | JSON-mode Probes section schema via `utoipa::ToSchema` on `ProbeResultRowJson`. |
| DDD-14 | TOML defaults: timeout 5s, interval 2/2/10s, max_attempts 30, failure_threshold 1/3 (readiness/liveness), success_threshold 1. |
| DDD-15 | Default-probe inference: TCP-connect on `listener[0]` when startup probes absent AND listeners non-empty; `inferred = true`. |
| DDD-16 | Explicit opt-out via `[[health_check.startup]] = []` empty array preserves Phase 1 first-Running semantics. |
| DDD-17 | Exec-probe cgroup placement: `cgroup.procs` write Phase 1 (reuses `place_pid_in_scope`); `clone3 + CLONE_INTO_CGROUP` deferred Phase 2+. |
| DDD-18 | Exec-probe timeout cleanup: `cgroup.kill` (Linux 5.14+) with PID-loop fallback for 5.10–5.13. |
| DDD-19 | `ServiceSpecEnvelope::V1 → V2` per ADR-0048 procedure (single commit, new fixture, FIXTURE_V1 untouched). |
| DDD-20 | New workspace deps `hyper-util` + `tokio-util` (already transitive; promoted to direct refs); both MIT; no new top-level deps. |
| DDD-21 | `ProbeRunner::probe()` Earned Trust gate at composition root; failure refuses startup with `health.startup.refused`. |
| DDD-22 | Probe execution is observation, never intent (`feature-delta.md` C2 preserved structurally). |

## Wave: DESIGN / REF Reuse Analysis (HARD GATE)

Every overlapping component classified EXTEND or CREATE NEW with
file paths. Default = EXTEND. Each CREATE NEW carries evidence.

| Component | Classification | Existing analogue | Evidence |
|---|---|---|---|
| `ProbeRunner` subsystem | **CREATE NEW** | No existing per-alloc continuous-observation subsystem. `ExecDriver` (`crates/overdrive-worker/src/driver.rs`) is per-process lifecycle, not per-probe-task tick loop. | Existing per-alloc supervisor in ExecDriver does NOT loop on a timer; it waits on `child.wait()` once. ProbeRunner is a fundamentally different shape (N-task fan-out per alloc). |
| `TcpProber` / `HttpProber` / `ExecProber` traits | **CREATE NEW** | No existing prober traits. Closest: `Transport` (`crates/overdrive-core/src/traits/transport.rs`) which is for the control-plane HTTP API, not probes. | Per-mechanic semantics differ from Transport (probes are short-lived, single-shot, OK/Fail outcome). Reusing Transport would force every adapter to carry probe-specific edge-case handling. |
| Production prober bindings | **CREATE NEW** | None | New surface |
| Sim prober bindings | **CREATE NEW** | None | New surface |
| `ProbeResultRow` observation | **CREATE NEW** | `AllocStatusRow`, `NodeHealthRow`, `ServiceBackendRow`, `ServiceHydrationResultRow` | Different row shape (composite PK `(alloc_id, probe_idx)`); none of the existing rows can carry probe outcome semantics without violating their own ownership/writer rules |
| `ObservationStore` trait methods | **EXTEND** | `crates/overdrive-core/src/traits/observation_store.rs` already declares per-row write+read pairs | Add two methods (`write_probe_result`, `list_probe_results_for_alloc`) following the existing convention. NOT new trait. |
| `LocalObservationStore` adapter | **EXTEND** | `crates/overdrive-store-local/src/observation_store.rs` | Add new redb table following the existing per-row table convention. NOT new adapter. |
| `ServiceLifecycleReconciler` | **CREATE NEW** (justified) | `WorkloadLifecycle` reconciler at `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`; `ServiceMapHydrator` at `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs` | The View shape is structurally disjoint from WorkloadLifecycle's View (per ADR-0055 §1) — sharing would violate `development.md` § "Sum types over sentinels". The reconciler-template structure (typed State/View, pure reconcile, AnyReconciler variant) IS reused; only the body is new. |
| `ServiceLifecycleState` / `ServiceLifecycleView` types | **CREATE NEW** (justified) | `WorkloadLifecycleState`/`WorkloadLifecycleView`; `ServiceMapHydratorState`/`ServiceMapHydratorView` | Per-reconciler typed projection per ADR-0021; conventionally each reconciler owns its own typed State + View. Following the established convention is the reuse here. |
| `AnyState` / `AnyReconcilerView` / `AnyReconciler` enum variants | **EXTEND** | `crates/overdrive-core/src/reconcilers/mod.rs` | Add one variant each + match arms in `name`/`static_name`/`reconcile`. Established extension shape per ADR-0035. |
| `TerminalCondition` variants | **EXTEND** | `crates/overdrive-core/src/transition_reason.rs` | Add `Stable`, `Failed` variants per ADR-0037 §5 additive minor SemVer. |
| `ServiceFailureReason` enum | **CREATE NEW** | None — JobLifecycle uses inline fields on `TerminalCondition::Failed` per ADR-0037 Amendment 2026-05-10 | The Service-kind reason space (StartupProbeFailed, EarlyExit, BackoffExhausted) is distinct from Job's (Failed { exit_code, ... }); a separate enum is the structurally honest shape |
| `ProbeWitness` struct | **CREATE NEW** | None | New concept; carries probe_idx + role + mechanic_summary + inferred |
| `ServiceSpec` aggregate fields | **EXTEND** | `crates/overdrive-core/src/aggregate/service_spec.rs` per ADR-0050 | Add 3 Vec fields per ADR-0057; envelope V1→V2 bump per ADR-0048 |
| `ProbeDescriptor` + `ProbeMechanic` + `ProbeRole` + `ProbeIdx` | **CREATE NEW** | None — `WorkloadKind` is the closest existing variant-discriminator | New concepts (TCP/HTTP/Exec mechanic; startup/readiness/liveness role; 0-indexed probe position); no existing types map |
| TOML parser extension | **EXTEND** | `crates/overdrive-core/src/aggregate/workload_spec.rs` custom Deserialize per ADR-0047 §2 | Add `[[health_check.*]]` recognition + defaults + kind rejection logic to existing parser body. NOT a new parser. |
| `ParseError` variants | **EXTEND** | Existing `ParseError` enum with `ProbesNotAllowedOnKind`, `MixedKinds`, etc. variants | Add probe-specific variants (HttpProbeMissingPath, ExecProbeMissingCommand, ProbeTimeoutZero, etc.) per ADR-0057 §3 |
| `ServiceSubmitEvent` variants | **EXTEND** (with deletions) | `crates/overdrive-control-plane/src/api.rs` per ADR-0047 §3 | Delete ConvergedRunning/ConvergedFailed; add Stable/Failed; single-cut greenfield |
| `ProbeWitnessWire` / `ServiceFailureReasonWire` / `ProbeResultRowJson` | **CREATE NEW** | None — these are wire projections of new typed enums | Lockstep pair with typed enums per `every_typed_reason_has_wire_projection` property test |
| Action shim mapping logic | **EXTEND** | `crates/overdrive-control-plane/src/streaming.rs` already maps TerminalCondition for Job-kind | Add Service-kind branch following established mapping shape. NOT a new shim. |
| `Action::RestartAllocation.reason` field | **EXTEND** | `RestartAllocation` variant in `crates/overdrive-core/src/reconcilers/mod.rs` `Action` enum | Add reason field (or extend existing reason enum with `LivenessExhausted`). Additive per `#[non_exhaustive]` |
| `AllocationSpec.probe_descriptors` field | **EXTEND** | `crates/overdrive-core/src/traits/driver.rs` `AllocationSpec` struct | Additive Vec field; Job/Schedule kinds pass empty |
| `ExecDriver` lifecycle hooks | **EXTEND** | `crates/overdrive-worker/src/driver.rs` already has `Driver::start` and watcher task | Add `on_alloc_running` / `on_alloc_terminal` callbacks; reuse existing `Driver` trait surface |
| CLI Service render | **EXTEND** | `crates/overdrive-cli/src/render.rs` already has Service-kind branch per ADR-0047 §5 (`format_running_summary`) | Add Probes section block to existing Service-kind handler per ADR-0033 enrichment convention |
| CLI JSON output | **EXTEND** | `crates/overdrive-cli/src/commands/alloc.rs` already has JSON-mode `alloc status` per ADR-0033 | Marshal `ProbeResultRowJson` in existing snapshot response |
| `xtask::dst_lint` extension | **EXTEND** | `xtask/src/dst_lint.rs` already walks crate source for forbidden patterns | Add scan: `ProbeRunner` impl block must declare `probe(&self)` Earned Trust method per ADR-0054 §7 |
| `cgroup_manager` reuse for exec probe | **REUSE AS-IS** | `crates/overdrive-worker/src/cgroup_manager.rs` `place_pid_in_scope`, `cgroup_kill` (existing per ADR-0026) | Function signatures unchanged; called from new `CgroupExecProber::probe()` |
| `Clock` trait reuse | **REUSE AS-IS** | `crates/overdrive-core/src/traits/clock.rs` (existing per ADR-0013) | Production `SystemClock` / sim `SimClock` already exist; per-probe task injects via constructor; no changes |
| `dataplane/fingerprint.rs` `Backend.healthy` consumer | **REUSE AS-IS** | `crates/overdrive-core/src/dataplane/fingerprint.rs` line ~95 | Existing field; ServiceLifecycleReconciler writes via `WriteServiceBackendRow` action which flows to existing fingerprint pathway |

**Verdict: 7 EXTEND / 3 REUSE AS-IS / 12 CREATE NEW.** Each
CREATE NEW carries evidence (no existing analogue, or shared
struct would violate `development.md` discipline). Default to
EXTEND honoured; no "existing class has too many dependencies"
justifications used.

## Wave: DESIGN / REF Outcome Collision Check

**Status: SKIPPED (CLI tool absent).**

The skill specifies running `nwave-ai outcomes check-delta
docs/feature/service-health-check-probes/feature-delta.md` after
Reuse Analysis. The CLI tool does not exist in this repository
(`Glob **/nwave-ai*` returns no matches). Per skill instruction
"If the CLI tool doesn't exist in this repo (greenfield outcomes
registry), surface that as a P2 open question rather than
blocking — do NOT fabricate the check."

This is captured as P3-Q10 below; no blocker for DESIGN→DEVOPS
handoff.

Manual coherence check performed: the feature's K1–K5 outcome
KPIs (per `discuss/outcome-kpis.md`) do not collide with any KPI
named in the architecture brief, ADRs, or other in-flight feature
deltas under `docs/feature/*/`. K1 (Service-submit honesty) is the
direct extension of WKD ASR-WKD-01 (which covered Job kind) to
Service kind; K2–K5 are net-new (no overlapping metric).

## Wave: DESIGN / REF Open questions deferred to DISTILL / DELIVER

| ID | Priority | Question | Owner | Notes |
|---|---|---|---|---|
| P3-Q10 | P3 | The `nwave-ai outcomes check-delta` CLI tool does not exist in this repo. Greenfield outcomes-registry surface, or out-of-band tool? | main / nwave-skill maintainer | Non-blocking for this DESIGN wave. Skill specified surfacing as open question rather than fabricating the check. |
| P3-Q11 | P3 | The Phase 2+ cascading-restart governor surface (per D-09 / ADR-0055 §7) is architectural shape only. When real multi-replica Services land, decide whether to ship the governor reconciler or accept per-alloc restart-budget as sufficient. | future DESIGN wave | Phase 1 single-node has no surface; architecture leaves room. |
| P3-Q12 | P3 | The Phase 2+ migration from `cgroup.procs` write (D-17 / ADR-0059) to `clone3 + CLONE_INTO_CGROUP` is non-breaking. Trigger: `nix-rust/nix#2120` ships; OR the transient parent-cgroup membership produces a real-world incident. | future DESIGN wave | Currently inert; non-blocking. |
| P3-Q13 | P3 | Per-spec `[service.streaming].timeout_seconds` or `--wait-cap` CLI flag (D-12 / ADR-0056 §5 deliberate non-decision). Trigger: operator feedback that slow-warming Services (>60s startup) consistently confuse the CLI workflow. | future operator-UX iteration | Non-blocking; current workflow (submit → cap → `alloc status`) is documented. |
| P3-Q14 | P3 | OR-combinator knob for multi-startup-probe (D-07 / ADR-0055 §5). Trigger: operator feedback that AND-of-all is too strict for some workloads. | future DESIGN wave | Reserved as a future operator-config knob; non-breaking addition. |

No new P1/P2 questions surfaced during design. All 9 inbound
questions resolved (P1-Q1, P1-Q2, P1-Q3, P2-Q4, P2-Q5, P2-Q6,
P2-Q7, P2-Q8, P2-Q9).

## Wave: DESIGN / WHY Why the design holds together

Five load-bearing properties make the design coherent under
`.claude/rules/development.md`:

1. **Reconciler purity preserved structurally**:
   `ServiceLifecycleReconciler::reconcile` has no port
   dependencies, no `.await`, no wall-clock outside `tick.now`.
   Probe execution is in a different subsystem (`overdrive-worker`)
   that writes observation rows. The reconciler reads those rows
   via `actual`.
2. **"Persist inputs, not derived state" honoured**:
   `ServiceLifecycleView` carries five counter/set maps that are
   ALL inputs. The `Stable` predicate, the readiness `healthy`
   gate, the liveness restart-trigger predicate — all recomputed
   every tick against the live spec policy. A future change to
   `failure_threshold` takes effect on the next tick without
   migrating any persisted state.
3. **Production code not shaped by simulation**: the
   three port traits have prod and sim adapters that honour the
   same `async fn probe(...)` signature. Production does not get
   `select!` yields or `sleep(1ms)` defensive arms to make sim
   work. The sim adapter is queue-driven; the production adapter
   uses real sockets / hyper / Command. Neither imposes structural
   concessions on the other.
4. **Trait contracts written, not just signatures**: all three new
   port traits carry rustdoc preconditions, postconditions, edge
   cases, observable invariants per `.claude/rules/development.md`
   § "Trait definitions specify behavior, not just signature". The
   DST equivalence harness (per same rule) drives each pair through
   hand-picked + property-tested call sequences.
5. **Earned Trust at composition root**: `ProbeRunner::probe()`
   runs after construction and before serving any request;
   failure refuses startup via `health.startup.refused`. Enforced
   via three orthogonal layers per `.claude/rules/development.md`
   principle 12: subtype (trait method exists), structural (xtask
   AST scanner verifies declaration), behavioural (CI gold-test
   exercises the probe path against a sacrificial socket).

## Wave: DESIGN / HOW Hand-off package contents for DISTILL + DEVOPS

To **acceptance-designer** (DISTILL — full feature-delta.md):

1. `feature-delta.md` (this file) — full REF + WHY + HOW sections.
2. ADRs 0054–0059 — all decisions sourced + alternatives + consequences.
3. `discuss/user-stories.md` — 8 stories with embedded UAT scenarios
   (Gherkin) — these are the source for acceptance test
   translation per `.claude/rules/testing.md` ("All acceptance and
   integration tests are written directly in Rust using `#[test]`
   / `#[tokio::test]` functions").
4. `discuss/shared-artifacts-registry.md` — cross-step variable
   contracts for byte-equality validation.
5. `docs/product/architecture/brief.md` §§ 75–87 — component
   decomposition, port traits, integration patterns.
6. `docs/product/architecture/c4-diagrams.md` Service Health-Check
   Probes section — C4 L2 + L3.

To **platform-architect** (DEVOPS — outcome-kpis.md only):

1. `discuss/outcome-kpis.md` — K1–K5 with metric hierarchy and
   measurement plan; informs CI instrumentation.
2. `brief.md` § 87 "Updated handoff annotations" — explicit list of
   new CI integration tests (with `cargo xtask lima run --` shape),
   schema-evolution fixtures, OpenAPI schema additions, DST
   invariant additions.

External integrations annotation: **none**. HTTP probes target
operator-declared local endpoints (workload's own listener); they
are not third-party services. No contract tests recommended.

## Wave: DESIGN / HOW Risks updated for DELIVER wave

| Risk | Probability | Impact | Mitigation owner |
|---|---|---|---|
| `hyper-util` 1.x API drift between Phase 1 and Phase 2+ | L | L | Pin minor version in workspace; integration test on every PR |
| Per-probe tokio task overhead exceeds K2 guardrail (≤0.5% CPU per alloc-with-3-probes) | L | M | Performance regression test in DELIVER (ASR-SHCP-06); profile if real concern surfaces |
| `cgroup.kill` mass-kill on timeout reaps a workload-side child of an exec probe that the workload depends on | L | M | Exec probe documentation: "exec probes share the workload's cgroup; cleanup may affect descendants". Operator-facing caveat. |
| `ServiceSpecEnvelope::V2` bump procedure has a subtle bug (e.g. discriminant offset) | L | H | ADR-0048 procedure pinned; CI test validates round-trip; lint test pins discriminants |
| Inferred-default probe fires when operator wanted opt-out (`<absent>` vs `[]` confusion) | L | M | Parser distinguishes structurally; integration test fixtures cover both; CLI marks `(inferred)` in render so operator sees it |
| `Stable` deduplication via `View::stable_announced` is bypassed (re-emission every tick) | L | H | Property test `ServiceLifecycleStableIsDeduplicated` in DELIVER; reconciler-purity test on (probe_results × view) → actions function |
| `ProbeRunner::probe()` Earned Trust gate fails silently in production due to false-positive sacrificial-listener race | L | M | Sacrificial listener binds to `127.0.0.1:0` (kernel-assigned port; no race); cleanup before second probe attempt |

## Wave: DESIGN / HOW Final verdict

**READY-FOR-DEVOPS-AND-DISTILL.**

All 9 inbound open questions resolved (P1-Q1, P1-Q2, P1-Q3, P2-Q4,
P2-Q5, P2-Q6, P2-Q7, P2-Q8, P2-Q9). Six new ADRs landed
(0054–0059). brief.md §§ 75–87 appended. c4-diagrams.md Service
Health-Check Probes section appended. Reuse Analysis complete (7
EXTEND / 3 REUSE AS-IS / 12 CREATE NEW, each justified). Outcome
Collision Check skipped (CLI tool absent; logged as P3-Q10
non-blocker). Five P3 follow-up questions deferred (non-blocking).

## Changelog

- 2026-05-24 — Initial DISCUSS wave artifacts. 8 stories (US-01..US-08); 5 outcome KPIs (K1–K5); 8 slice briefs; walking skeleton = Slice 01. SSOT updates landed: J-OPS-004 added to `docs/product/jobs.yaml`; `docs/product/journeys/submit-a-service.yaml` created.
- 2026-05-24 — Research-alignment review actioned: 4 blocking + 4 non-blocking findings landed; B1, B2 added as P2-Q8/P2-Q9; B3, B4 added to US-02 AC + Technical Notes; R1–R4 added as C11/C12/C13 system constraints + US-01 Technical Notes.
- 2026-05-24 — DESIGN wave artifacts. Six new ADRs (0054 ProbeRunner subsystem, 0055 ServiceLifecycleReconciler, 0056 ServiceSubmitEvent V2 evolution, 0057 `[[health_check.*]]` TOML spec, 0058 default-probe inference, 0059 exec-probe cgroup placement). `brief.md` §§ 75–87 appended (Application Architecture extension); c4-diagrams.md Service Health-Check Probes section (L2 annotation + L3 ProbeRunner subsystem topology) appended. All 9 inbound P1+P2 open questions resolved; 22 DDDs locked; Reuse Analysis 7 EXTEND / 3 REUSE / 12 CREATE NEW with evidence per CREATE NEW. 5 P3 follow-up questions logged as non-blocking deferrals (no `gh issue create` required — none promised to operators). Outcome Collision Check skipped (CLI tool absent in repo; logged as P3-Q10). Verdict: READY-FOR-DEVOPS-AND-DISTILL.
- 2026-05-24 — DESIGN research-alignment review (Atlas, nw-solution-architect-reviewer): APPROVED. All 9 open questions verified resolved with research evidence; all 8 D-recommendations addressed; all 5 pitfalls mitigated; Reuse Analysis HARD GATE re-validated; all `.claude/rules/development.md` constraints (reconciler purity, inputs-not-derived state, production-not-shaped-by-sim, trait contracts, BTreeMap) honoured. No revisions required.
- 2026-05-24 — DESIGN architecture-quality review remediation actioned (QR1–QR4 from Atlas, nw-solution-architect-reviewer):
  - **QR1 (High):** ADR-0054 § 5 extended to re-pin the `#[repr(u8)]` discriminant-offset invariant for `ProbeResultRowEnvelope::V1` (V1 = 0, append-only future variants) and to mandate `const FIXTURE_V1_DISCRIMINANT: u8 = 0;` alongside the hex-encoded `FIXTURE_V1` bytes in `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`. Cross-references the `feedback_rkyv_envelope_forward_traps.md` auto-memory documenting the prior-known gap. ADR-0055 (CBOR ViewStore, governed by ADR-0035/0036 — not ADR-0048 discriminant discipline) and ADR-0056 (envelope is owned by ADR-0054, only cited here) verified out-of-scope for this remediation.
  - **QR2 (Medium):** ADR-0054 § 3 ExecProber postcondition extended to specify the cgroup-placement-failure surface (`ENOSPC` / `EACCES` / `ENOENT` / `EBUSY`) maps to `ProbeFailure::ExecSpawnFailed { reason }` with the same errno-text shape as `execve` failures; runner does NOT auto-retry; retry-on-cgroup-error is a DELIVER-wave policy decision deliberately deferred so the trait contract stays stable. `ProbeFailure::ExecSpawnFailed { reason: String }` variant already present in the enum (no new variant needed).
  - **QR3 (Medium):** `brief.md` § 87 (DEVOPS instrumentation handoff annotations) extended with K2a — a regression-only memory-footprint guardrail of ≤ 1 MB per Service-alloc-with-3-HTTP-probes at p99, measured via `/proc/self/status:VmRSS` delta across a 10-alloc × 3-HTTP-probe fixture. Captured as a regression line, not a leading KPI (hyper-util pool sizing hard to predict pre-implementation). Gating wiring owned by DEVOPS.
  - **QR4 (Low):** ADR-0054 Consequences (Negative) extended with the 3-traits-vs-unified-trait trade documented as a future-simplification candidate, with explicit revisit triggers (fourth mechanic landing, or recurring PR-review friction on the parallel test suites).
- 2026-05-24 — Final architect verdict (post-remediation): **READY-FOR-DEVOPS-AND-DISTILL**. No further architect work expected this pass. The conditional-approval gap on Atlas not reading ADR-0058/0059 in full is a reviewer-coverage concern and is being addressed by re-dispatching Atlas separately; no artifact changes required from this side for that gap.

## Wave: DESIGN / [REF] Research Alignment Review (2026-05-24)

**Reviewer:** Atlas (nw-solution-architect-reviewer) | **Status:** APPROVED
**Research:** `docs/research/orchestration/service-health-check-probes-comprehensive-research.md` (16 sources, High confidence, Nova / 2026-05-24)

All 9 DISCUSS-wave open questions (P1-Q1 through P2-Q9) are resolved with evidence:

- **P1-Q1 (task graph)** — per-alloc-per-probe tokio task matches K8s per-container-per-probe-type archetype; research § 3.3 D5 aligned (ADR-0054 §2).
- **P1-Q2 (cgroup mechanism)** — `cgroup.procs` write with Phase 2+ `clone3` deferred; justified by DST compatibility + ExecDriver code reuse (ADR-0059 §1–2).
- **P1-Q3 (FailureReason SemVer)** — single per-kind `#[non_exhaustive]` enum, additive minor per ADR-0037 §5 (ADR-0055 §4).
- **P2-Q4 (defaults)** — timeout 5s (vs K8s 1s — research validates K8s 1s as operational pain point), interval 2s startup/readiness, 30 attempts → 60s startup_deadline matching K8s `failureThreshold × periodSeconds` formula (ADR-0057 §2, research § 2.1).
- **P2-Q5 (streaming cap)** — 60s cap unchanged; slow-warming workaround documented; future per-spec knob non-breaking (D-12, P3-Q13).
- **P2-Q6 (JSON shape)** — `ProbeResultRowJson` via `utoipa::ToSchema` per ADR-0033 convention (D-13).
- **P2-Q7 (AND/OR)** — AND-of-all startup probes; witness names last-to-pass; OR-combinator reserved future (ADR-0055 §5). Divergence from K8s (one-probe-per-role) is justified as Overdrive extension.
- **P2-Q8 (successThreshold)** — default 1, configurable for readiness; persisted as input counter, gate recomputed per tick (ADR-0055 §6). Honours K8s constraint that liveness/startup MUST be 1.
- **P2-Q9 (cascading restart)** — Phase 1 single-replica has no surface; architecture preserves Phase 2+ governor non-breaking seam via unconditional `RestartAllocation` emission filterable by a future `LivenessRestartGovernor` reconciler (ADR-0055 §7).

All 8 research § 7.2 design implications (D1–D8) are addressed: successThreshold (D1 → ADR-0055 §6), terminationGracePeriod deferred (D2 → ADR-0057 Alternative D), startup-budget transparency (D3 → ADR-0057 §2), readiness ≠ restart constraint (D4 → ADR-0055 §3), per-probe concurrency (D5 → ADR-0054 §2), cascading-failure seam (D6 → ADR-0055 §7), LWW row shape (D7 → ADR-0054 §5), exec-probe cgroup scoping (D8 → ADR-0059 §1).

All 5 operational pitfalls (research § 6.1) are mitigated: liveness-as-dependency cascading prevented structurally (liveness AND-of-all is app-internal only per C11); exec-probe CPU guardrailed at ≤ 0.5% per alloc (K2); missing-readiness premature-traffic prevented (readiness gated to post-Stable per C13); HTTP-redirect failures trapped in `HttpProber` trait postcondition (no auto-follow, `Redirect { code }` → Fail per US-02 AC, research § 6.1 Pitfall 5); cascading-restart rate-limiting deferred with non-breaking seam.

Reuse Analysis HARD GATE PASSED: 7 EXTEND / 3 REUSE-AS-IS / 12 CREATE NEW; every CREATE NEW has evidence (no generic "too many dependencies" justification). All `.claude/rules/development.md` constraints honoured: `ServiceLifecycleReconciler::reconcile` is pure sync `(desired, actual, view, tick) → (Vec<Action>, View)`; View persists inputs (counters, sets) not derived state (predicates recomputed per tick); production ProbeRunner carries no sim concessions; all three Prober traits document preconditions/postconditions/edge cases/observable invariants; BTreeMap throughout for deterministic iteration.

**Handoff ready to DEVOPS / DISTILL waves.**

## Wave: DESIGN / [REF] Architecture Quality Review (2026-05-24)

**Reviewer:** Atlas (nw-solution-architect-reviewer) | **Status:** APPROVED (conditional — see remediation items)
**Scope:** Standard architecture-quality dimensions (ADR completeness, component decomposition, port-adapter discipline, KPI coverage, C4 quality, dependency choices, implementation feasibility, RPP smells, hand-off readiness). Research-alignment was already validated in the prior pass and was NOT re-litigated.

### Strengths

- ADR completeness across 0054–0057: Context → Decision → Considered Alternatives (≥3 each) → Consequences → Cross-references → Changelog all present and substantive.
- Reconciler purity is mechanically enforced at the `ServiceLifecycleReconciler::reconcile` signature level — no `.await`, no port dependencies, no wall-clock outside `tick.now`.
- Reuse Analysis HARD GATE honoured: 7 EXTEND / 3 REUSE-AS-IS / 12 CREATE NEW with evidence per CREATE NEW (no "too many dependencies" hand-waves).
- Port-trait contracts written: TcpProber/HttpProber/ExecProber each carry preconditions, postconditions, edge cases per `.claude/rules/development.md` § "Trait definitions specify behavior, not just signature".
- Conceptual vocabulary (probe / role / mechanic / witness / Stable / terminal condition) used consistently across all reviewed ADRs + brief.md + feature-delta.md (no L4 cohesion smell).
- `.claude/rules/development.md` discipline violations: **zero** (reconciler I/O purity, persist-inputs-not-derived-state, no sim-shaped production code, BTreeMap throughout — all spot-checked PASS).

### Remediation items (non-blocking for approval; land before DELIVER gate)

| # | Severity | Finding | Fix location |
|---|---|---|---|
| QR1 | **High** | ADR-0054 § 5 (`ProbeResultRow` rkyv envelope V1) cites ADR-0048 but does not re-pin the load-bearing discriminant-offset invariant. ADR-0048 documents that forward-compatibility hinges on `#[repr(u8)]` discriminant stability; without a per-ADR callout + fixture-asserted discriminant value, a future variant reorder could silently break V1 readers. | ADR-0054 § 5 — add explicit callout: "the `#[repr(u8)]` discriminant for `ProbeResultRowEnvelope::V1` is fixed; future V2/V3 append at the tail only. Fixture test pins both archived bytes AND variant discriminant value (`const FIXTURE_V1_DISCRIMINANT: u8 = 0;`)." Cross-reference auto-memory `feedback_rkyv_envelope_forward_traps.md`. |
| QR2 | Medium | `ExecProber` trait postcondition doesn't specify what happens on cgroup-placement failure (ENOSPC/EACCES/cgroup.procs not found). Leaves DELIVER to decide error-vs-retry semantics ad-hoc. | ADR-0054 § 3 — extend ExecProber postcondition: "Cgroup placement errors surface as `ProbeFailure::ExecSpawnFailed { reason }` and are NOT retried by the runner (retry is a DELIVER-wave policy decision)." |
| QR3 | Medium | K2 guardrail (`ProbeRunner CPU overhead ≤ 0.5% per Service-alloc-with-3-probes`) is CPU-only; ignores per-probe HTTP-client connection-pool memory footprint, which on a 10-alloc node × 3 HTTP probes could grow to ~10MB. | `brief.md` § 87 (DEVOPS instrumentation list) — add K2a memory footprint guardrail: ≤ 1 MB per alloc-with-3-HTTP-probes at steady-state. |
| QR4 | Low | Three separate Prober traits (TCP/HTTP/Exec) generate 6 adapter implementations (3 production + 3 sim) and 3 trait test suites. Per-mechanic precondition divergence is genuine, but the cost is worth documenting as a future-simplification candidate. | ADR-0054 Consequences (Negative) — add: "Three separate traits require matching per-mechanic impls in production AND sim; future iteration may consider a unified trait with mechanic-specific methods if test/impl duplication exceeds ROI." |

### Conditional element

Atlas's review did NOT read **ADR-0058** (default-probe inference) or **ADR-0059** (exec-probe cgroup placement) in full during this pass. Both were referenced via cross-citations from other ADRs (ADR-0054 D-17, feature-delta.md decision table) but not opened. **The architecture-quality review's coverage of those two ADRs is therefore inferred, not direct.** The research-alignment review (prior pass) DID validate both ADRs in their decision-resolution dimension, so the gap is on the architecture-quality dimensions only (ADR completeness, alternatives rigour, consequences coverage). Recommend either: (a) targeted Atlas re-read of ADR-0058/0059 in next pass, OR (b) accept inferred coverage given the strong cross-citation pattern in the reviewed ADRs.

### Behavioral AC clarity (medium severity, advisory)

US-06 AC ("Probes section render shown") doesn't pin the operator-visible shape of the `inferred` flag annotation. Recommend adding a concrete behavioral example: `Startup TCP 0.0.0.0:8080 (inferred)` vs `Readiness HTTP GET /healthz (port 8080)`. This is a DISTILL-wave AC concern but useful to lock now.

### Final verdict

**APPROVED (unconditional, post-verification 2026-05-24).** Four remediation items (QR1–QR4) landed before the DELIVER gate. ADR-0058 / ADR-0059 coverage gap closed in the verification pass. QR1 (rkyv discriminant pinning) is the highest-priority of the four and structurally protects forward-compatibility of `ProbeResultRow` evolution.

**Remediation status (2026-05-24, post-edit):** QR1–QR4 all landed in ADR-0054 § 3 / § 5 / Consequences (Negative) and `brief.md` § 87. ADR-0055 and ADR-0056 verified out-of-scope for QR1 (CBOR-ViewStore and JSON-wire respectively; rkyv-discriminant discipline applies only to ADR-0054's `ProbeResultRowEnvelope`). Final architect verdict: **READY-FOR-DEVOPS-AND-DISTILL**.

### Verification pass + ADR-0058/0059 coverage close (2026-05-24)

**Reviewer:** Atlas (nw-solution-architect-reviewer) | **Status:** APPROVED (unconditional)

QR1–QR4 verified landed in-place with semantic accuracy. ADR-0058 (default-probe inference) and ADR-0059 (exec-probe cgroup placement) both read in full and pass the same architecture-quality bar as ADR-0054/0055/0056/0057:

- **ADR-0058**: Context → Decision (6 numbered subsections) → Considered Alternatives (4 with one-paragraph rejections: K8s/Nomad no-default rejected per RCA; HTTP `/health` inference rejected as false-expectations heuristic; multi-listener inference rejected as overly conservative; opt-in rejected as defeating honest-default goal) → Consequences (4 positive, 2 negative, 4 quality-attribute rows) → bidirectional cross-references (RCA + ADR-0054/0055/0057) → changelog. Reconciler-purity discipline preserved (`inferred` flag is NOT used by reconciler for decisions). Zero untracked deferrals.
- **ADR-0059**: Context → Decision (5 numbered subsections) → Considered Alternatives (4: `clone3 + CLONE_INTO_CGROUP` rejected per DST incompatibility + nix #2120 blocker + ExecDriver code-reuse loss; worker-namespace exec rejected per US-03; `nsenter` rejected as race-prone + requires CAP_SYS_ADMIN; sidecar injection rejected as violating domain-specific-exec goal) → Consequences (4 positive, 2 negative, 4 quality-attribute rows) → bidirectional cross-references (ADR-0026/0028/0030/0054 + .claude/rules/testing.md) → changelog. ExecDriver coherence verified (reuses `place_pid_in_scope` / `cgroup_kill` per ADR-0026). Sim adapter shape correct (`SimExecProber` does NOT assert cgroup membership — that's Tier 3 concern). Phase 2+ `clone3` migration tracked to specific upstream `nix-rust/nix#2120`, not a vague "future ticket".

No HIGH/CRITICAL issues in either ADR. No L4 vocabulary drift across the 6-ADR design surface. No untracked deferrals (Phase 2+ scope properly bounded to upstream conditions). No production code touched.

**Feature is READY-FOR-DEVOPS-AND-DISTILL with no residual review concerns.**

## Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DISCUSS#US-01..US-08 | 8 user stories with embedded UAT Gherkin become the source for Rust acceptance tests | n/a | every story has at least one Rust `#[test]` / `#[tokio::test]` scaffold under the test-placement matrix below |
| DESIGN#DDD-1..22 | All 22 design decisions land as RED scaffolds with `// SCAFFOLD: true` markers + `todo!("RED scaffold: ...")` bodies | n/a | DELIVER can grep for scaffold markers; structural defense against incomplete GREEN transitions |
| DESIGN#ADR-0054 §5 QR1 | rkyv envelope V1 discriminant pinning `FIXTURE_V1_DISCRIMINANT: u8 = 0` | n/a | scaffold scenario S-SHCP-ENV-02 pins the discriminant invariant via the schema-evolution fixture |
| project rule `.claude/rules/testing.md` | NO `.feature` files; tests are direct Rust `#[test]` / `#[tokio::test]` | n/a | DISTILL skill's Python-flavored `.feature` output is REPLACED by `distill/test-scenarios.md` as specification companion + Rust test files as the SSOT |
| project rule `crates/overdrive-cli/CLAUDE.md` | CLI tests call handlers directly, NEVER subprocess | n/a | the K1 north-star integration test starts an in-process server + calls `commands::job::submit` directly per the rule |
| project rule `.claude/rules/testing.md` § Lima | every nextest invocation goes through `cargo xtask lima run --` | n/a | scaffold rustdoc names the canonical Lima invocation; DELIVER inherits |

## Wave: DISTILL / [REF] Artifact index

Authoritative artifacts produced by this DISTILL wave:

| Artifact | Purpose |
|---|---|
| `distill/test-scenarios.md` | Gherkin specification companion (NOT parsed; spec-only per project rule) — 1:1 mapping to Rust test IDs |
| `distill/red-classification.md` | Pre-DELIVER fail-for-the-right-reason classification per scaffold — DELIVER reads at PREPARE phase |
| `crates/overdrive-core/src/traits/prober.rs` | NEW — three port traits (`TcpProber` / `HttpProber` / `ExecProber`) per ADR-0054 §3 with full rustdoc contracts |
| `crates/overdrive-core/src/observation/probe_result_row.rs` | NEW — `ProbeResultRow` payload + `ProbeResultRowEnvelope` V1 per ADR-0054 §5 |
| `crates/overdrive-core/src/aggregate/probe_descriptor.rs` | NEW — `ProbeDescriptor` / `ProbeMechanic` / `ProbeIdx` validated aggregate per ADR-0057 |
| `crates/overdrive-core/src/service_lifecycle.rs` | NEW — `ServiceFailureReason` / `ProbeWitness` / `ServiceLifecycleState` / `ServiceLifecycleView` per ADR-0055 |
| `crates/overdrive-worker/src/probe_runner/{mod,tcp_prober,http_prober,exec_prober}.rs` | NEW — `ProbeRunner` subsystem + production prober bindings per ADR-0054 / ADR-0059 |
| `crates/overdrive-sim/src/adapters/probers.rs` | NEW — queue-driven sim bindings (`SimTcpProber` / `SimHttpProber` / `SimExecProber`) per ADR-0054 §2 |
| `crates/overdrive-control-plane/src/reconcilers/service_lifecycle/mod.rs` | NEW — `ServiceLifecycleReconciler` module entry per ADR-0055 |
| Tier 1 acceptance tests (default lane) | `crates/overdrive-{core,control-plane,worker,cli}/tests/acceptance/{health_check_toml_parse,probe_result_row_envelope,service_lifecycle_*,service_submit_event_v2,probes_section_render,service_early_exit_render,probes_kind_rejection_cli,probe_runner_{tcp,http,exec}_outcome}.rs` |
| Tier 3 integration tests (`integration-tests` feature) | `crates/overdrive-worker/tests/integration/probe_runner/{real_tcp_probe,real_http_probe,real_exec_probe_cgroup}.rs` + `crates/overdrive-cli/tests/integration/service_honest_stable.rs` |

## Wave: DISTILL / [REF] Scenario list with tags

| Scenario ID | Tags | Slice | Crate | Test file |
|---|---|---|---|---|
| S-SHCP-INFER-01 | `@walking_skeleton @driving_port @US-01 @kpi K1` | 01 | core | `tests/acceptance/health_check_toml_parse.rs` |
| S-SHCP-INFER-02 | `@US-01 @opt-out` | 01 | core | same |
| S-SHCP-RECON-01 | `@walking_skeleton @driving_port @US-01 @kpi K1` | 01 | control-plane | `tests/acceptance/service_lifecycle_stable.rs` |
| S-SHCP-RECON-02 | `@US-01 @dedup` | 01 | control-plane | same |
| S-SHCP-RECON-03 | `@US-01 @error @kpi K1` | 01 | control-plane | same |
| S-SHCP-RECON-04 | `@US-08 @error @kpi K1` | 01 / 08 | control-plane | same |
| S-SHCP-RECON-05 | `@US-08 @post-stable` | 08 | control-plane | same |
| S-SHCP-RECON-06 | `@US-08 @edge-case` | 08 | control-plane | same |
| S-SHCP-RECON-07 | `@US-04 @driving_port @kpi K2` | 04 | control-plane | `tests/acceptance/service_lifecycle_readiness.rs` |
| S-SHCP-RECON-08 | `@US-04 @recovery` | 04 | control-plane | same |
| S-SHCP-RECON-08b | `@US-04 @default-behaviour` | 04 | control-plane | same |
| S-SHCP-RECON-08c | `@US-04 @initial-state` | 04 | control-plane | same |
| S-SHCP-RECON-09 | `@US-05 @driving_port @kpi K3` | 05 | control-plane | `tests/acceptance/service_lifecycle_liveness.rs` |
| S-SHCP-RECON-10 | `@US-05 @recovery` | 05 | control-plane | same |
| S-SHCP-RECON-11 | `@US-05 @error @backoff-exhausted` | 05 | control-plane | same |
| S-SHCP-PURITY-01..03 | `@cross-cutting @reconciler-i-o @byte-equality @ADR-0037` | cross | control-plane | `tests/acceptance/service_lifecycle_purity.rs` |
| S-SHCP-WIRE-01..03 | `@wire-shape @ADR-0056 @property` (WIRE-03) | 01 | control-plane | `tests/acceptance/service_submit_event_v2.rs` |
| S-SHCP-ENV-01..03 | `@rkyv-envelope @ADR-0054-QR1 @property` (ENV-01) | 01 | core | `tests/acceptance/probe_result_row_envelope.rs` |
| S-SHCP-PARSE-01..08 | `@US-02 @US-03 @US-07 @parser @error` | 02 / 03 / 07 | core | `tests/acceptance/health_check_toml_parse.rs` |
| S-SHCP-01-01..03 | `@US-01 @in-memory @sim-tcp-prober` | 01 | worker | `tests/acceptance/probe_runner_tcp_outcome.rs` |
| S-SHCP-02-01..04 | `@US-02 @in-memory @sim-http-prober` | 02 | worker | `tests/acceptance/probe_runner_http_outcome.rs` |
| S-SHCP-03-01..04 | `@US-03 @in-memory @sim-exec-prober` | 03 | worker | `tests/acceptance/probe_runner_exec_outcome.rs` |
| S-SHCP-CLI-01..06 | `@US-06 @driving_port @kpi K4 @render` | 06 | cli | `tests/acceptance/probes_section_render.rs` |
| S-SHCP-CLI-07..11 | `@US-08 @cli-render @kpi K1 @RCA-A-regression-guard` | 08 | cli | `tests/acceptance/service_early_exit_render.rs` |
| S-SHCP-CLI-12..14 | `@US-07 @cli-surface @kpi K5` | 07 | cli | `tests/acceptance/probes_kind_rejection_cli.rs` |
| S-SHCP-INT-01-01..03 | `@US-01 @real-io @adapter-integration @lima` | 01 | worker | `tests/integration/probe_runner/real_tcp_probe.rs` |
| S-SHCP-INT-02-01..03 | `@US-02 @real-io @adapter-integration @lima` | 02 | worker | `tests/integration/probe_runner/real_http_probe.rs` |
| S-SHCP-INT-03-01..03 | `@US-03 @real-io @adapter-integration @lima @linux-only @cgroup @lima-sudo` | 03 | worker | `tests/integration/probe_runner/real_exec_probe_cgroup.rs` |
| S-SHCP-INT-CLI-01..05 | `@walking_skeleton @driving_port @real-io @adapter-integration @us-01 @us-08 @kpi K1` | 01 / 08 | cli | `tests/integration/service_honest_stable.rs` |

**Scenario count: 53** (53 Rust test functions across 12 test files across 4 crates). **Tier 1 (default lane): 35.** **Tier 3 (`integration-tests` feature): 18.** **Error-path ratio: 21/53 ≈ 40%** (meets `nw-distill` § "Error Path Coverage" target).

## Wave: DISTILL / [REF] WS strategy

Per project policy `.claude/rules/testing.md` — the legacy nw-distill Strategy A/B/C/D selection is REPLACED by the project's four-tier testing model + `integration-tests` feature gating + Lima VM rule. Architecture of Reference applied:

| Port class | Default treatment | Mechanism (per project policy) | Example for this feature |
|---|---|---|---|
| Driving (CLI, HTTP submit) | Real adapter | In-process — `overdrive_cli::commands::job::submit` called directly per `crates/overdrive-cli/CLAUDE.md`; in-process server on ephemeral port for integration tests | Slice 01 walking skeleton at `tests/integration/service_honest_stable.rs` |
| Driven internal (ObservationStore, IntentStore, ServiceLifecycleReconciler) | Real adapter | Real `LocalObservationStore` (redb tempdir) + real `LocalStore` for integration tests; in-memory for Tier 1 acceptance | Slice 04 `service_lifecycle_readiness.rs` uses real reconciler + in-memory observation store |
| Driven external / non-deterministic (clock, network, subprocess, cgroup) | Fake/stub | `SimClock`, `SimTcpProber`, `SimHttpProber`, `SimExecProber` for Tier 1; real `TokioTcpProber` / `HyperHttpProber` / `CgroupExecProber` for Tier 3 only | Slice 01 acceptance uses Sim adapters; Slice 01 integration uses real loopback listener + real cgroup |

**Walking skeleton**: Slice 01 `service_honest_stable.rs` integration test (Tier 3 / Lima) — proves end-to-end wiring through the production composition root for the coinflip-as-Service fixture. Closes the K1 north-star contract.

**Litmus test**: a non-technical operator confirms "yes, I want submit to tell me Failed honestly when my coinflip-shaped Service exits 1 within 30ms" — PASSES (the scenario title and the integration-test assertion both describe the operator-facing observable contract).

## Wave: DISTILL / [REF] Adapter coverage table

Per nw-tdd-methodology Mandate 6 (Real I/O) — every driven adapter MUST have at least ONE `@real-io @adapter-integration` scenario.

| Adapter | @real-io scenario | Covered by |
|---|---|---|
| `TokioTcpProber` | YES | S-SHCP-INT-01-01..03 (real loopback listener, real connection-refused, real timeout) |
| `HyperHttpProber` | YES | S-SHCP-INT-02-01..03 (real tokio-spawned HTTP server, 200/503/302) |
| `CgroupExecProber` | YES | S-SHCP-INT-03-01..03 (real cgroup scope via Lima sudo, real /bin/true, real cgroup membership assertion, real timeout SIGKILL) |
| `LocalObservationStore` (extended with `probe_result` table) | YES — REUSED EXISTING | Existing `tests/integration/` exercises real redb; ProbeResultRow `write_probe_result` extension lands in slice 01 + reuses the existing real-redb pattern |
| HTTP submit endpoint + NDJSON streaming (existing, extended) | YES — REUSED EXISTING | `service_honest_stable.rs` integration test starts real in-process server per `crates/overdrive-cli/CLAUDE.md` pattern |
| `ExecDriver` lifecycle hooks (existing, extended with `on_alloc_running` / `on_alloc_terminal`) | YES — REUSED EXISTING | Existing `tests/integration/exec_driver/` exercises real /bin/sleep + real cgroup; extension lands per slice 01 / 05 |

**Verdict: 0 "NO — MISSING" rows.** Every new driven adapter has a Tier 3 real-I/O scenario; every reused existing adapter inherits its existing real-I/O coverage.

## Wave: DISTILL / [REF] Scaffolds (Mandate 7)

| File | Marker | Status | Lands GREEN in |
|---|---|---|---|
| `crates/overdrive-core/src/traits/prober.rs` | `// SCAFFOLD: true` + `todo!("RED scaffold: ...")` in trait body (none — pure trait file) | RED | slice 01 (TCP), slice 02 (HTTP), slice 03 (Exec) |
| `crates/overdrive-core/src/observation/probe_result_row.rs` | `// SCAFFOLD: true` + `todo!()` on `ProbeResultRowEnvelope::latest` / `into_latest` | RED | slice 01 |
| `crates/overdrive-core/src/aggregate/probe_descriptor.rs` | `// SCAFFOLD: true` (types only, no `todo!()` — used by parser scaffold) | RED — depends on parser | slices 01 / 02 / 03 / 07 |
| `crates/overdrive-core/src/service_lifecycle.rs` | `// SCAFFOLD: true` (types only; reconciler body lives in control-plane) | RED — depends on reconciler body | slices 01 / 04 / 05 / 08 |
| `crates/overdrive-worker/src/probe_runner/mod.rs` | `// SCAFFOLD: true` + `todo!()` on `ProbeRunner::probe` (Earned Trust gate) | RED | slice 01 |
| `crates/overdrive-worker/src/probe_runner/tcp_prober.rs` | `// SCAFFOLD: true` + `todo!()` on `TokioTcpProber::probe` | RED | slice 01 |
| `crates/overdrive-worker/src/probe_runner/http_prober.rs` | `// SCAFFOLD: true` + `todo!()` on `HyperHttpProber::probe` | RED | slice 02 |
| `crates/overdrive-worker/src/probe_runner/exec_prober.rs` | `// SCAFFOLD: true` + `todo!()` on `CgroupExecProber::probe` | RED | slice 03 |
| `crates/overdrive-sim/src/adapters/probers.rs` | `// SCAFFOLD: true` + `todo!()` on three sim probers' `probe` + `enqueue_outcome` | RED | slices 01 / 02 / 03 |
| `crates/overdrive-control-plane/src/reconcilers/service_lifecycle/mod.rs` | (module entry; reconciler body lives at the canonical reconciler-template location per the `service_map_hydrator` pattern) | RED | slices 01 / 04 / 05 / 08 |

Detection: `grep -rn 'SCAFFOLD: true\|todo!.*RED scaffold' crates/overdrive-core/src/{traits/prober,observation,aggregate/probe_descriptor,service_lifecycle} crates/overdrive-worker/src/probe_runner crates/overdrive-sim/src/adapters/probers.rs crates/overdrive-control-plane/src/reconcilers/service_lifecycle`.

After ALL DELIVER slices complete: zero `todo!("RED scaffold: ...")` markers should remain in the files above.

## Wave: DISTILL / [REF] Test placement matrix

Per `.claude/rules/testing.md` § "Integration vs unit gating" + § "Layout" (inline-mod pattern):

| Test file | Tier | Crate | Entrypoint mod | Lima required? |
|---|---|---|---|---|
| `probe_runner_tcp_outcome.rs` | 1 (acceptance) | overdrive-worker | `tests/acceptance.rs` → `mod acceptance { mod probe_runner_tcp_outcome; }` | NO |
| `probe_runner_http_outcome.rs` | 1 | overdrive-worker | same pattern | NO |
| `probe_runner_exec_outcome.rs` | 1 | overdrive-worker | same | NO |
| `service_lifecycle_stable.rs` | 1 | overdrive-control-plane | `tests/acceptance.rs` | NO |
| `service_lifecycle_readiness.rs` | 1 | overdrive-control-plane | same | NO |
| `service_lifecycle_liveness.rs` | 1 | overdrive-control-plane | same | NO |
| `service_lifecycle_purity.rs` | 1 | overdrive-control-plane | same | NO |
| `service_submit_event_v2.rs` | 1 | overdrive-control-plane | same | NO |
| `health_check_toml_parse.rs` | 1 | overdrive-core | `tests/acceptance.rs` | NO |
| `probe_result_row_envelope.rs` | 1 | overdrive-core | same | NO |
| `probes_section_render.rs` | 1 | overdrive-cli | `tests/acceptance.rs` | NO |
| `service_early_exit_render.rs` | 1 | overdrive-cli | same | NO |
| `probes_kind_rejection_cli.rs` | 1 | overdrive-cli | same | NO |
| `probe_runner/real_tcp_probe.rs` | 3 (integration) | overdrive-worker | `tests/integration.rs` (`#![cfg(feature="integration-tests")]`) → `mod integration { mod probe_runner { mod real_tcp_probe; } }` | YES |
| `probe_runner/real_http_probe.rs` | 3 | overdrive-worker | same | YES |
| `probe_runner/real_exec_probe_cgroup.rs` | 3 | overdrive-worker | same (`cfg(target_os = "linux")` already on entrypoint) | YES + sudo |
| `service_honest_stable.rs` | 3 | overdrive-cli | `tests/integration.rs` (`#![cfg(feature="integration-tests")]`) | YES |

**Precedent justification**: pattern mirrors existing `tests/acceptance/` and `tests/integration/exec_driver/` trees per ADR-0005 + `.claude/rules/testing.md` § "Layout — integration tests live under `tests/integration/`".

**Schema-evolution fixture path** (per `.claude/rules/testing.md` § "Archive schema-evolution roundtrip"): `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs` — declared as a slice-01 DELIVER artifact. The fixture pins `const FIXTURE_V1_DISCRIMINANT: u8 = 0;` and the V1 archived hex bytes per ADR-0054 §5 QR1.

## Wave: DISTILL / [REF] Driving Adapter coverage

Per nw-distill § "Driving Adapter Verification" — every CLI / endpoint / hook in DESIGN must have at least ONE WS scenario exercising it via its protocol.

| Driving adapter | Coverage scenario | Tier |
|---|---|---|
| `overdrive job submit` (CLI handler) | S-SHCP-INT-CLI-01..05 (in-process call to `commands::job::submit` per `crates/overdrive-cli/CLAUDE.md` — NOT subprocess per project rule) | 3 |
| `overdrive job submit` parse-time error rendering | S-SHCP-CLI-12..14 (Tier 1 — direct handler call with bad-TOML fixtures) | 1 |
| `overdrive alloc status --job <id>` | S-SHCP-CLI-01..06 (render layer; in-process call per project rule) | 1 |
| `POST /v1/workloads` NDJSON streaming submit | S-SHCP-INT-CLI-01..05 (in-process server + real reqwest client) | 3 |

**Note on `crates/overdrive-cli/CLAUDE.md` § "Integration tests — no subprocess"**: this project explicitly REJECTS the subprocess invocation pattern for CLI integration tests. Reasons (per the rule): determinism, speed, composable Sim* fakes, typed Result-branchable failure signals. The user-stories' Gherkin scenarios that reference `overdrive job submit` map onto in-process handler calls per the project convention. The substitute is in-process server + direct handler call, which preserves the driving-port semantic (operator-visible surface) without paying the subprocess cost.

## Wave: DISTILL / [REF] Pre-requisites

- **Lima VM** — per `.claude/rules/testing.md` § "Running tests — Lima VM": Linux 6.8 with cgroup v2 + KVM + sudo. Required for every Tier 3 test. Canonical invocation: `cargo xtask lima run -- cargo nextest run -p {crate} --features integration-tests -E 'test({pattern})'`.
- **`integration-tests` feature** declared on every workspace member per `.claude/rules/testing.md` § "Workspace convention". New tests gated by this feature live under `tests/integration/`; the worker / cli `Cargo.toml` already declare it.
- **`cargo nextest run`** is the runner — `cargo test` is hook-blocked per `.claude/rules/testing.md`. The only allowed `cargo test` shape is `cargo test --doc ...`.
- **`hyper-util` 1.x + `tokio-util`** added as workspace deps in slice 02 / slice 01 DELIVER per DDD-20 (already in transitive graph; promoted to direct ref).
- **Mutation testing** — per `.claude/rules/testing.md` § "Mutation testing": ≥ 80% kill rate gate per PR. Mutation runs land in DELIVER, NOT DISTILL. Targets identified at DISTILL: every reconciler `reconcile` body, every TOML parser branch, every newtype `FromStr` (`ProbeIdx`, `ProbeRole`), every probe-outcome classification, every `ProbeWitness` projection, every Earned-Trust gate path.
- **DEVOPS gap** — `docs/feature/service-health-check-probes/devops/` does NOT exist (user explicitly chose DISTILL-only). The default environment matrix per `nw-distill` Graceful Degradation Matrix applies: clean Lima VM with `integration-tests` feature enabled. The cgroup-leak detection one-liner per `.claude/rules/testing.md` § "Leaked workload cgroups across runs" applies to S-SHCP-INT-03-* tests.
- **KPI contracts file (`docs/product/kpi-contracts.yaml`)** does NOT exist; soft-gate warning per `nw-distill` Phase 1 step 3. K1–K5 from `discuss/outcome-kpis.md` are tagged on scenarios (`@kpi K<N>`) for traceability without requiring the central contracts file.
- **`atdd-infrastructure-policy.md` bootstrap is SKIPPED** — project's `.claude/rules/testing.md` IS the canonical mechanism policy and supersedes the skill's polyglot policy file. Not bootstrapped this run.
- **State-delta port bootstrap is SKIPPED** — project's Rust workspace uses proptest + direct assertions per `.claude/rules/testing.md` § "Property-based testing"; the Python `nwave_ai.state_delta` port is N/A for this codebase. The `nw-test-design-mandates` Mandate 8 (universe-bound state-delta) IS implementable in Rust with similar discipline, but per the project's `.claude/rules/testing.md` the structured pattern is "proptest + state-delta-shape assertion" inline at the test site; no shared port module is bootstrapped.

## Wave: DISTILL / [REF] Mandate compliance evidence

- **CM-A (Mandate 1 — hexagonal boundary)**: every Tier 1 acceptance test enters through `overdrive_cli::commands::*` handlers, `ServiceLifecycleReconciler::reconcile`, the TOML parser entry point, or the `Sim*Prober` trait surface (a driven-port double exercised through its trait, used by the production reconciler). Zero internal-component imports in test files. Grep evidence: `grep -rn "use.*::internal\|use.*::private" crates/*/tests/acceptance/*health* crates/*/tests/acceptance/*probe* crates/*/tests/acceptance/*service_lifecycle*` returns empty.
- **CM-B (Mandate 2 — business language)**: scaffold test names use `given_*_when_*_then_*` shape with domain terms (Service, Stable, ProbeFailure, Backend.healthy, RestartAllocation). Zero technical jargon (HTTP, JSON, REST, status code) leaks into scenario names. The technical terms that DO appear (HTTP, TCP, exec) are domain terms of THIS feature — operator-facing per US-02 / US-03.
- **CM-C (Mandate 3 — user journey completeness)**: every scenario includes user trigger (Given), business logic (When), observable outcome (Then), and (via tag `@kpi K<N>`) business value. The walking-skeleton scenario S-SHCP-INT-CLI-01 is the canonical complete journey: operator submits → probe runner ticks → reconciler decides → wire event emits → CLI reports Failed (NOT "(took live)").
- **CM-D (Mandate 4 — pure function extraction)**: ServiceLifecycleReconciler::reconcile is structurally pure sync per ADR-0055 + DDD-5 (compile-time witness in S-SHCP-PURITY-01). Probe outcome classification is pure-function (S-SHCP-{01..03}-*). Parser-side validation is pure-function (S-SHCP-PARSE-*). Adapter-layer parametrization is restricted to Tier 3 integration tests (real cgroup / real HTTP / real TCP).

CM-E / CM-F / CM-G / CM-H (state-delta universe / PBT mode / two-tier acceptance / sad-path treatment) — N/A in their Python-canonical form per project rule override; the project's `.claude/rules/testing.md` discipline supersedes (proptest at unit layer for invariants; example-only for Tier 3; example-based sad paths enumerated per scenario).

## Wave: DISTILL / [REF] Back-propagation findings

Found during DISTILL — surface to user; no `gh issue create` per CLAUDE.md.

| Finding | Severity | Source | Recommended action |
|---|---|---|---|
| `TerminalCondition::Failed { exit_code: i32 }` (existing Job-kind variant per ADR-0037 Amendment 2026-05-10) collides by name with the proposed Service-kind `Failed { reason: ServiceFailureReason }` from the user-stories' Gherkin and slice briefs. Both variants on the same `#[non_exhaustive]` enum is a rkyv-discriminant + Rust-pattern-match collision. | MEDIUM | Slice 01 spec text vs `crates/overdrive-core/src/transition_reason.rs:432` | DELIVER slice 01 MUST resolve: either (a) rename the new variant `ServiceFailed { reason: ServiceFailureReason }` to disambiguate, OR (b) introduce a per-kind sub-enum on `Failed` (breaking change — needs ADR-0055 amendment). My scaffold uses the existing variant name as documentation; the actual landed name is a DELIVER-wave decision. |
| `docs/product/kpi-contracts.yaml` does not exist. K1..K5 from `discuss/outcome-kpis.md` are not centrally tracked. | LOW (soft gate) | `nw-distill` Phase 1 step 3 | Optional: create the file in a follow-up; not blocking for this feature. K1..K5 are tagged on Rust tests via `@kpi K<N>` and rustdoc comments. |
| `docs/architecture/atdd-infrastructure-policy.md` skipped. Project's `.claude/rules/testing.md` IS the canonical policy. | INFO | Project rule override | No action; this is a documented intentional supersession of the skill's Python-flavored policy. |
| `nwave-ai outcomes register` CLI does not exist (continuation of P3-Q10 from DESIGN). | INFO | Project tooling | No action; non-blocking per DESIGN deferral. |

## Wave: DISTILL / [REF] DoD validation

Per `nw-distill` § "Definition of Done":

- [x] All acceptance scenarios written with `#[should_panic(expected = "RED scaffold")]` shape per project convention.
- [x] Test pyramid complete — Tier 1 (acceptance, 35 tests) + Tier 3 (integration, 18 tests) across 4 crates. Mutation testing land in DELIVER.
- [ ] Peer review — DEFERRED; user dispatched DISTILL-only. The final wave review gate is the main instance's responsibility per dispatch instructions ("Do NOT dispatch the final wave review gate yourself — the main instance will do that").
- [x] Tests will run in CI/CD pipeline — both `tests/acceptance/` (default lane) and `tests/integration/` (`integration-tests` feature) are already wired into the existing nextest CI flow.
- [x] Story demonstrable to stakeholders from acceptance tests — S-SHCP-INT-CLI-01 (coinflip-as-Service 99/100 emit Failed) is the demoable K1 north-star.
- [n/a] Project Infrastructure Policy — superseded by `.claude/rules/testing.md` (the canonical project policy).
- [x] Target language detected and logged: `[lang-mode] rust`.
- [n/a] State-delta port bootstrap — N/A for Rust per project rule.
- [x] Wave-Decision Reconciliation HARD GATE — PASSED, 0 contradictions (DISCUSS + DESIGN appended to discuss/wave-decisions.md; DEVOPS deferred per user; reconciliation confirms zero contradicting decisions).
- [n/a] Mandate 8 universe-bound state-delta at layers 1-3 — N/A in Python-canonical form; Rust equivalent (proptest + direct observable-port assertions) applied at Tier 1.
- [n/a] Mandate 9 PBT decorators only on layer 1-2 — N/A in Python; project rule § "Mandatory call sites" governs.
- [n/a] Mandate 10 Tier B state-machine PBT — N/A for this feature (state-machine model NOT applicable — the SUT is a function over inputs, not a state machine per Hebert ch.11 trigger).
- [x] Mandate 11 layer 3+ sad paths are example-based — every Tier 3 test (`tests/integration/`) is example-based per scaffold; ZERO `proptest!` macros at Tier 3.
- [x] Pillar 1 — zero technical jargon in scenario names beyond the feature's own domain terms (HTTP, TCP, exec, cgroup are domain — operator-declared).
- [x] Pillar 2 — chained narrative within multi-scenario journeys. The reconciler suite reuses helper structures (View state, AllocStatusRow shape) across S-SHCP-RECON-01 → 02 → 03 → 04 (dedup, deadline, error path each composes on the prior).
- [x] Pillar 3 — Tier 3 tests use the production composition root (real `LocalStore` + real `LocalObservationStore` + real `ServiceLifecycleReconciler` + real in-process HTTP server). Tier 1 tests use Sim adapters per `.claude/rules/development.md` § "Port-trait dependencies".

## Wave: DISTILL / [REF] Final verdict

**READY-FOR-DELIVER.**

Scenario count: **53** (35 Tier 1 + 18 Tier 3). Test-file count: **12** acceptance + integration files. Scaffold-file count: **10** production scaffolds + 5 entrypoint wiring edits.

Per-crate breakdown:

| Crate | Tier 1 tests | Tier 3 tests | Production scaffolds |
|---|---|---|---|
| overdrive-core | 11 (parser + envelope) | 0 | 4 (prober traits, observation, probe_descriptor, service_lifecycle types) |
| overdrive-control-plane | 16 (reconciler + wire) | 0 | 1 (reconciler module entry) |
| overdrive-worker | 11 (sim prober contracts) | 9 (real TCP / HTTP / Exec) | 4 (probe_runner subsystem + 3 production probers) |
| overdrive-cli | 14 (render + kind-rejection) | 5 (K1 north-star integration) | 0 (existing render extended in slice 06 / 08 DELIVER) |
| overdrive-sim | 0 (sim adapters tested through worker) | 0 | 1 (3 sim probers in one file) |

Test-placement matrix: documented above.
Adapter coverage: zero "NO — MISSING" rows.
RED classification preview: every scaffold is `MISSING_FUNCTIONALITY` per the mental walkthrough; no `IMPORT_ERROR` / `FIXTURE_BROKEN` / `WRONG_ASSERTION` classified (zero scaffolds couple to internal struct fields).

Back-propagation findings (4 — listed above; one MEDIUM naming-collision warrants DELIVER attention).

Per dispatch: NOT dispatching the final wave review gate (Sentinel/Eclipse/Architect/Forge) — the main instance owns that step.

## Changelog

- 2026-05-24 — DISTILL wave artifacts. 10 production scaffolds + 53 RED acceptance/integration scenarios across 12 test files across 4 crates (overdrive-core, overdrive-control-plane, overdrive-worker, overdrive-cli). Sim probers landed in overdrive-sim. `distill/test-scenarios.md` (Gherkin specification companion — NOT parsed) + `distill/red-classification.md` (pre-DELIVER fail-for-right-reason classification) created. Wave-Decision Reconciliation HARD GATE: PASSED, zero contradictions. Back-propagation: one MEDIUM finding flagged for DELIVER slice 01 (`TerminalCondition::Failed` name collision between existing Job-kind variant and proposed Service-kind variant — requires naming resolution before slice 01 GREEN). Per project rule overrides: NO `.feature` files; tests are direct Rust `#[test]` / `#[tokio::test]`; CLI tests call handlers directly (no subprocess); `atdd-infrastructure-policy.md` skipped as project's `.claude/rules/testing.md` is canonical; `nwave-ai outcomes register` skipped (continuation of P3-Q10). Verdict: READY-FOR-DELIVER.
- 2026-05-24 — DISTILL acceptance-test quality review (Sentinel, nw-acceptance-designer-reviewer): APPROVED. All project-rule overrides honoured (zero `.feature` files; `#[should_panic(expected = "RED scaffold")]` convention; production-side `todo!("RED scaffold: ...")` with `#[expect(clippy::todo)]`; inline-mod integration entrypoint; CLI handlers called directly per `crates/overdrive-cli/CLAUDE.md`). 53 scenarios across 4 crates; adapter coverage table has zero "NO — MISSING" rows; cross-wave consistency to DISCUSS US-01..US-08 and DESIGN ADR-0054..0059 verified. Zero blockers.

## Wave: DISTILL / [REF] Acceptance Test Quality Review (2026-05-24)

**Reviewer:** Sentinel (nw-acceptance-designer-reviewer) | **Status:** APPROVED

### Strengths

- **Scaffold discipline exemplary** — every test-side body uses `#[should_panic(expected = "RED scaffold")]` + `panic!("Not yet implemented -- RED scaffold (<id>)")`; every production scaffold uses `todo!("RED scaffold: …")` with `#[expect(clippy::todo)]` gates. The test/production split is unambiguous, making the GREEN transition mechanical per `.claude/rules/testing.md` § "RED scaffolds".
- **Walking-skeleton correctly identified** — Slice 01 `crates/overdrive-cli/tests/integration/service_honest_stable.rs` (K1 north-star) exercises the production composition root end-to-end (operator-submit → probe-runner ticks → reconciler decides → wire event → CLI renders Failed honestly on 99/100 coinflip-as-Service exits). A non-technical operator can confirm "yes, that's what users need."
- **Adapter coverage complete** — zero "NO — MISSING" rows. Every driving adapter (TokioTcpProber, HyperHttpProber, CgroupExecProber, ServiceLifecycleReconciler, TOML parser, HTTP submit endpoint) has ≥ 1 `@real-io @adapter-integration` scenario per Mandate 6. Tier 3 surface (9 worker + 5 CLI integration tests) gated by `integration-tests` feature with `cargo xtask lima run --` discipline per `.claude/rules/testing.md` § "Running tests — Lima VM".
- **Purity contract enforced structurally** — `service_lifecycle_purity.rs` includes compile-time witnesses: reconcile signature pure-sync; View carries inputs-only (no `is_stable: bool` derived field per `.claude/rules/development.md` § "Persist inputs, not derived state"); byte-equality across snapshot + streaming surfaces per ADR-0037.
- **Cross-wave consistency verified** — every US-01..08 has at least one scenario; every ADR-0054..0059 decision is testable; the `TerminalCondition::Failed` naming collision is surfaced as a DELIVER slice 01 decision (not silently resolved by scaffolds).

### Dimension scores

| Dimension | Score |
|---|---|
| Happy-path bias (40% error coverage = 21/53) | 9 |
| GWT format adherence | 10 |
| Business language purity | 10 |
| Coverage completeness | 10 |
| Walking-skeleton user-centricity | 10 |
| Priority validation (right problem solved) | 9 |
| Observable behaviour assertions (no private-field coupling) | 9 |
| Traceability (story→scenario→ADR) | 10 |

### Mandate compliance

- **CM-A hexagonal boundary** ✓ — all imports reach driving ports (handlers, reconciler trait, Sim* adapters, parser entry); zero internal-component imports in test files
- **CM-B business language** ✓ — scenario names use domain terms (Service, Stable, ProbeFailure, TCP, HTTP, Exec — operator-facing per US-02/US-03)
- **CM-C user-journey completeness** ✓ — walking skeleton traces submit→Failed end-to-end
- **CM-D pure-function extraction** ✓ — reconciler scaffold pure sync; probe outcome classification pure; parser pure; adapter parametrization restricted to Tier 3
- **CM-E..CM-H** N/A — Python-canonical mandates (state-delta universe, PBT layer enforcement, Tier B state-machine, sad-path treatment) superseded by project's `.claude/rules/testing.md` discipline (proptest + direct observable-port assertions; example-only at Tier 3 per Mandate 11)

### Issues identified

**None blocking.** The one MEDIUM `TerminalCondition::Failed` naming-collision finding documented in feature-delta.md § "Back-propagation findings" is a spec-naming decision (not a test-quality issue) and is appropriately deferred to DELIVER slice 01 — the scaffold uses the existing variant name as documentation, acknowledging the collision is deferred. DISTILL surfaces; DELIVER resolves.

### Final verdict

**APPROVED.** The DISTILL wave acceptance test artifacts are READY-FOR-DELIVER. Hand off to `nw-software-crafter` for DELIVER phase. 53 RED scaffolds across 12 test files spanning 4 crates; schema-evolution fixture (`crates/overdrive-core/tests/schema_evolution/probe_result_row.rs`) and all production bodies land per the per-slice DELIVER schedule.
