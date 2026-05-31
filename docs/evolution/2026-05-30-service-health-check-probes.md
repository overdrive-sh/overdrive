# Evolution — service-health-check-probes

**Finalised:** 2026-05-30
**Feature ID:** `service-health-check-probes`
**Waves:** DISCUSS → (DESIGN, in-ADR) → DISTILL → DELIVER
**Source brief:** GitHub issue #170 (supersedes #169, settle-window primitive rejected)
**Predecessor RCA:** `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`
**Job:** J-OPS-004 (Service-honesty; extends J-OPS-003)
**Steps:** 18/18 DONE

---

## 1. Summary + business context

A declarative health-check primitive for the **Service** workload kind:
HTTP / TCP / Exec probe mechanics, scoped per role (startup / readiness /
liveness), k8s-shape `[[health_check.*]]` TOML, with **honest-by-default**
inference. The feature closes **RCA root cause A**: *kernel-accepted exec
is NOT operator-meaningful liveness*.

The originating failure (2026-05-09): an operator ran
`overdrive job submit examples/coinflip.toml` and watched the CLI print
`is running with 1/1 replicas (took live)` with exit code 0 — even though
the workload was alternately exiting 0 and 1 within ~30ms. The platform was
reporting the kernel's bare-fork acceptance as if it were liveness. The
operator's only workaround was `submit && sleep N && alloc status`, which is
unscriptable and gives false confidence when `N` is too small.

The north-star contract (KPI **K1**): a Service whose workload exits `1`
within the startup deadline (the reshaped coinflip case) must emit
`ServiceSubmitEvent::Failed { reason: EarlyExit { exit_code: 1 } }` on
**≥99 of 100** deterministic trials, with **zero** `Stable` terminals and
**zero** occurrences of the literal `(took live)` string in any rendered
event. Baseline was 0% (Phase 1 always reported a Stable-equivalent for the
kernel-accepted window).

### Outcome KPIs (from `discuss/outcome-kpis.md`)

| # | Tier | Contract |
|---|---|---|
| **K1** | North Star | Service-submit honesty rate: `Failed { StartupProbeFailed \| EarlyExit }` (never `Stable`, never bare `Running`) within `startup_deadline + 1 tick`, ≥99/100 |
| K2 | Leading | Readiness Pass→Fail flips `Backend.healthy = false` in the dataplane fingerprint within 1 reconciler tick |
| K3 | Leading | Liveness consecutive fails past threshold → `RestartAllocation` within 1 tick (consumes shared restart budget) |
| K4 | Leading | `alloc status --job <service-id>` renders a Probes section (Service only; absent for Job/Schedule) |
| K5 | Guardrail | `[[health_check.*]]` on `[job]`/`[schedule]` → parse-time `ProbesNotAllowedOnKind { kind, guidance }` naming the right primitive |

---

## 2. Key decisions

DESIGN-wave decisions live permanently in the ADRs under
`docs/product/architecture/` (this project keeps ADRs there, not in a
feature `design/` directory). The six probe ADRs:

- **ADR-0054** — ProbeRunner subsystem (per-alloc supervisor + per-probe
  tokio task; matches K8s `prober.Manager` shape; Earned-Trust `probe()`
  gate). `docs/product/architecture/adr-0054-prober-subsystem.md`
- **ADR-0055** — ServiceLifecycleReconciler (readiness `successThreshold`
  counter persisted as input; liveness `failure_threshold`; restart emitted
  unconditionally while budget remains; cascading-restart governor deferred
  to Phase 2). `docs/product/architecture/adr-0055-service-lifecycle-reconciler.md`
- **ADR-0056** — ServiceSubmitEvent `Stable` / `Failed` evolution
  (`#[non_exhaustive]`, additive variants; wire projection kept in lockstep
  with `AllocStatusRow.terminal` via property test).
  `docs/product/architecture/adr-0056-service-submit-event-evolution.md`
- **ADR-0057** — `[[health_check.*]]` TOML spec (timeout 5s, intervals
  2/2/10s, max_attempts 30, failure_threshold 1/3, success_threshold 1;
  plain HTTP only). `docs/product/architecture/adr-0057-health-check-toml-spec.md`
- **ADR-0058** — Default-probe inference ("honest by default": no
  `[[health_check.startup]]` + ≥1 `[[listener]]` infers a TCP-connect
  startup probe at `probe_idx = 0`; empty array is the explicit opt-out).
  `docs/product/architecture/adr-0058-default-probe-inference.md`
- **ADR-0059** — Exec-probe cgroup placement (`cgroup.procs` write reusing
  `place_pid_in_scope`; `clone3 + CLONE_INTO_CGROUP` deferred pending
  `nix-rust/nix#2120`).
  `docs/product/architecture/adr-0059-exec-probe-cgroup-placement.md`

> **Awareness — ADR-0059 number collision (pre-existing, not fixed here).**
> Two files both claim `adr-0059`:
> `docs/product/architecture/adr-0059-exec-probe-cgroup-placement.md` and
> `docs/product/architecture/adr-0059-service-submit-event-terminal-taxonomy.md`.
> This is a pre-existing follow-up: ADR edits go through the architect agent
> only, so this finalize dispatch does NOT renumber either file. Flagged for
> a future architect pass.

Supporting decisions (`discuss/wave-decisions.md`): all 9 DISCUSS open
questions (P1-Q1…P2-Q9) resolved before DELIVER; five P3 deferrals surfaced
(none promised to operators, no GH issues required); brief.md §§ 75–87 and
c4-diagrams.md Service Health-Check Probes section appended.

---

## 3. Steps completed (18/18, from `deliver/execution-log.json`)

The roadmap deliberately carries **suffix step IDs** (`01-03b`,
`01-03f-1`, `01-03e3-fix`, …) — the audit trail in the execution-log `sid`s
and 11 commits' `Step-ID:` trailers depends on these exact IDs. They are
intentional, not a defect.

| Step | What landed |
|---|---|
| 01-01 | Foundation types: Prober trait surface, `ProbeResultRowEnvelope` V1, `ProbeDescriptor` aggregate, `ServiceLifecycleState`/`View`, `TerminalCondition` variants |
| 01-02 | TOML parser `[[health_check.startup]]` (TCP) + default-TCP inference |
| 01-03 | `ServiceLifecycleReconciler` core reconcile body (Stable / StartupProbeFailed / EarlyExit branches) — checkpoint, later re-sliced |
| 01-03b | `AnyReconciler` dispatch wiring (ServiceLifecycle variants + cross-crate match arms) |
| 01-03c | ProbeRunner subsystem + `TokioTcpProber` + Earned-Trust `probe()` gate + dst_lint clause + obs-store probe-row extension |
| 01-03d | ExecDriver lifecycle hooks + `AllocationSpec.probe_descriptors` + composition-root Earned-Trust wiring |
| 01-03e | `ServiceSubmitEvent` V1→V2 (types only) |
| 01-03e2 | Variant + projection taxonomy (PBT lockstep guards) |
| 01-03e3 | Dispatch wiring for the V2 projection |
| 01-03e3-fix | Lockstep-projection follow-up |
| 01-03f-1 | Activate S-SHCP-INT-CLI-02/03/04/05 end-to-end (quick-bind Stable; never-binds StartupProbeFailed; snapshot↔streaming byte-equality; composition-root probe gate) |
| 01-03f-2 | K1 100-seed coinflip regression guard (S-SHCP-INT-CLI-01) |
| 01-03f | Umbrella close for the 01-03 family |
| 02-01 | HTTP startup probe — `HyperHttpProber` + TOML `http` variant (GET only, 3xx→Fail, no redirect-follow) |
| 02-02 | Exec startup probe in-cgroup — `CgroupExecProber` (reuses `place_pid_in_scope` + `cgroup.kill`) |
| 02-03 | CLI Probes-section render (TUI + JSON), kind-guarded Service-only |
| 03-01 | Kind-rejection on Job/Schedule (K5) + readiness flips `Backend.healthy` (K2) |
| 03-02 | Liveness → `RestartAllocation` (K3) + RCA-A render hardening |

Key DELIVER commits: `2fabf259` (01-03 checkpoint), `c2d99561`/`dc9687f6`
(01-03f-1 activation), `fc75457a` (K1 guard, 01-03f-2), `14ba3741` (02-01
HTTP), `6e113184` (02-02 Exec), `73259b2a` (02-03 render), `12338b28`
(03-01), `c5505c9f` (03-02).

---

## 4. Lessons learned — the Phase-01 structural-gap cascade

> This is the load-bearing lesson of the feature. It is the reason every
> future "X is wired into Y" step in this repo should carry an acceptance
> test authored at the **wiring-claim** granularity.

### 4.1 What happened

Phase 01 shipped **green tests at every layer** while the Service probe
subsystem was **structurally dead end-to-end**. Each step's acceptance test
exercised its unit in isolation (or hand-assembled X+Y inside a test
fixture), CI was green, and the roadmap marched on — but no test asserted
that the **production composition root** actually constructs and connects
the pieces the way production runs. A `.context/01-03-structural-gap-audit.md`
pass (2026-05-28) found **5 distinct structural gaps**; closing them
end-to-end surfaced **eleven** (GAP-1…GAP-11).

### 4.2 Root cause (from `.context/01-03-structural-gap-audit.md`)

> *Acceptance tests are authored at the granularity of the
> unit-under-implementation, not at the granularity of the
> unit-under-wiring-claim.* A step that claims "X is wired into Y" gets an
> AT that exercises X-in-isolation, Y-in-isolation, or X+Y assembled by
> hand in a fixture — but **no AT that asserts the production composition
> root constructs X+Y the way production runs.**

The worst offender was **01-03d** ("composition-root Earned-Trust
wiring"): its AT exercised `compose_and_probe_runner_gate(...)` in
isolation against Sim adapters, while the production root at `lib.rs:1015`
discarded the returned `Arc<ProbeRunner>` into `let _probe_runner = …` and
constructed `ExecDriver::new(...)` **without** `.with_probe_runner(...)`.
The probe subsystem was never connected; every lifecycle hook took the
trait-default no-op path. A composition-root *claim* demands a
composition-root *test*; the AT lived one layer below the claim.

### 4.3 The eleven gaps (the dead chain, made live)

The end-to-end chain
`probe runs → ProbeResultRow written → hydrate projects Pass → reconciler emits Stable`
was broken at multiple arrows simultaneously:

- **GAP-1** (`d6ef5aa9`) — `hydrate_desired`/`hydrate_actual` returned
  `ServiceLifecycleState::default()` placeholders; the reconciler saw empty
  state forever. Replaced with real 3-source joins (alloc_status row +
  ProbeResultRow LWW + ServiceSpec from IntentStore).
- **GAP-3** (`cf3787f4`) — dst_lint enforced probe-method *declaration* but
  not its *call site*; added absence-checks for the production probe-gate
  invocation points.
- **GAP-4/5** (`86dbdab9`) — production `ExecDriver` built before the
  ProbeRunner gate ran and the runner was discarded; new
  `compose_production_driver(...)` runs the gate first, then threads the
  runner via `with_probe_runner`. New dst_lint clause rejects
  `let _probe_runner` bindings.
- **GAP-6** (`8afebdde`) — TOML-parsed probe vecs were dropped between CLI
  submit and intent admission (`ServiceSpecInput` and `ServiceV1` had no
  probe fields); persisted them end-to-end through wire + rkyv.
- **GAP-7** (`1dc54f91`) — `ProbeRunner::start_alloc` registered a
  supervisor but never spawned per-descriptor tick tasks; added the
  production per-descriptor execution loop, so rows actually get written.
- **GAP-8** (`81780380`) — the shared reconciler hardcoded
  `probe_descriptors: Vec::new()` for both Job and Service; Service-kind now
  projects its descriptors into `AllocationSpec` via
  `project_probe_descriptors`.
- **GAP-9/10** (`a84a1b5a`) — the `service-lifecycle` reconciler was
  registered but had zero production enqueue paths and was never re-ticked;
  added initial-wakeup dual-emit + a self-sustaining re-enqueue predicate
  (`has_alloc_mid_startup_window`) that keeps it alive through the startup
  window and flips false the instant a terminal is reached.
- **GAP-11** (`fe6b7374`) — `hydrate_service_alloc_facts` hardcoded
  `exit_code: None` behind an unbacked "downstream slice" deferral comment,
  so every EarlyExit reported `exit_code: 0`; now sourced from
  `row.reason = WorkloadCrashedImmediately { exit_code }`, mirroring the
  Job-kind precedent at `workload_lifecycle.rs:944`. This was the seam K1
  needed: before the fix K1 measured 0/100 `EarlyExit{exit_code:1}`; after
  it, the exit code carries end-to-end.

### 4.4 The fix pattern

- Each gap closed as its **own focused commit** (so the audit trail names
  exactly what each seam was and how it was sealed).
- The **K1 walking-skeleton** (`service_honest_stable.rs`, S-SHCP-INT-CLI-01)
  is the test that **spans the whole chain** — real CLI handler → in-process
  server → reconciler → real `ExecDriver` → ProbeRunner → observation store.
  It is the structural defense that prevents the class from silently
  recurring: a future regression that disconnects any arrow turns the
  100-seed loop red.
- dst_lint gained call-site absence-checks (GAP-3, GAP-4/5) so the
  "declared but never invoked" and "constructed but discarded" shapes fail
  at PR time rather than shipping green.

### 4.5 Two non-blocking items surfaced at 03-02 (awareness, not blockers)

1. **Pre-existing `render.rs:327` `cli_error_to_exit_code` mutant** — a
   missed mutant on the Slice 07 render surface. The 03-02 mutation gate
   still passed at **96.7%** (well above the ≥80% threshold); the mutant
   pre-dates this feature's render touch and is a Slice 07 surface concern.
2. **EarlyExit elapsed/deadline render-side recompute** — the rendered
   `elapsed`/`startup_deadline` are recomputed at render time from persisted
   inputs rather than persisted as derived values. This is **correct** per
   `.claude/rules/development.md` § "Persist inputs, not derived state" —
   noted so a future reader does not mistake it for a missing field.

---

## 5. Links to migrated artifacts (Phase B destinations)

Lasting artifacts copied to permanent directories (workspace originals
preserved):

- UX journey:
  - `docs/ux/service-health-check-probes/journey-service-honest-stable.yaml`
  - `docs/ux/service-health-check-probes/journey-service-honest-stable-visual.md`
- Scenarios + slice specs:
  - `docs/scenarios/service-health-check-probes/test-scenarios.md`
  - `docs/scenarios/service-health-check-probes/slice-01-walking-skeleton-default-tcp-startup.md`
  - `docs/scenarios/service-health-check-probes/slice-02-http-startup-probe.md`
  - `docs/scenarios/service-health-check-probes/slice-03-exec-startup-probe.md`
  - `docs/scenarios/service-health-check-probes/slice-04-readiness-flips-backend-healthy.md`
  - `docs/scenarios/service-health-check-probes/slice-05-liveness-triggers-restart.md`
  - `docs/scenarios/service-health-check-probes/slice-06-cli-probes-section.md`
  - `docs/scenarios/service-health-check-probes/slice-07-reject-probes-on-job-schedule.md`
  - `docs/scenarios/service-health-check-probes/slice-08-early-exit-detection.md`

ADRs 0054–0059 are already permanent in `docs/product/architecture/` (not
migrated, not duplicated). Research is already permanent in
`docs/research/orchestration/service-health-check-probes-comprehensive-research.md`.
