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

## Changelog

- 2026-05-24 — Initial DISCUSS wave artifacts. 8 stories (US-01..US-08); 5 outcome KPIs (K1–K5); 8 slice briefs; walking skeleton = Slice 01. SSOT updates landed: J-OPS-004 added to `docs/product/jobs.yaml`; `docs/product/journeys/submit-a-service.yaml` created.
- 2026-05-24 — Research-alignment review actioned: 4 blocking + 4 non-blocking findings landed; B1, B2 added as P2-Q8/P2-Q9; B3, B4 added to US-02 AC + Technical Notes; R1–R4 added as C11/C12/C13 system constraints + US-01 Technical Notes.
