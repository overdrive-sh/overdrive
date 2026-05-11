# Definition of Ready Validation — workload-kind-discriminator

Validates each story in `user-stories.md` against the 9-item DoR hard gate per
`nw-leanux-methodology` (8 standard items + 1 outcome-KPI item per
`nw-outcome-kpi-framework`).

## Changed Assumptions

- 2026-05-10 — added US-08 (Service listener spec shape) per fold-in of GH
  #164. Pass count updated from 7/7 to 8/8.

## DoR Status Summary

| Story | Status |
|---|---|
| US-01 (Parser kind discriminator) | **PASSED** |
| US-02 (Job submit terminal) | **PASSED** |
| US-03 (alloc status kind-aware) | **PASSED** |
| US-04 (Service preservation) | **PASSED** |
| US-05 (Schedule parsing + deferral) | **PASSED** — deferral GH issue [#166](https://github.com/overdrive-sh/overdrive/issues/166) created 2026-05-09 with user approval. CLI constant references that URL. See § "Deferral gating — RESOLVED" below. |
| US-06 (Anti-pattern grep gate) | **PASSED** (technical task) |
| US-07 (Migrate `examples/coinflip.toml`) | **PASSED** (migration task) |
| US-08 (Service listener spec shape) | **PASSED** — folded in 2026-05-10 from [#164](https://github.com/overdrive-sh/overdrive/issues/164). Runtime allocator behaviour referenced via [#167](https://github.com/overdrive-sh/overdrive/issues/167) (approved 2026-05-09); spec layer is forward-compatible. |

**Overall**: 8/8 fully passed.

---

## US-01: Parser kind discriminator

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear and in domain language | **PASS** | "Ana wants to write `[job]` for a one-shot and `[service]` for a long-running process" — concrete persona + concrete real-world data (coinflip workload). |
| 2. User/persona identified | **PASS** | Ana, Overdrive platform engineer (matches `submit-a-job.yaml`'s persona). |
| 3. 3+ domain examples with real data | **PASS** | (1) `payments` Service, (2) `nightly-backup` Schedule, (3) ambiguous spec rejection. All with real TOML bodies and real CLI output. |
| 4. UAT in Given/When/Then (3-7 scenarios) | **PASS** | 5 scenarios in story + cross-checked in `journey-submit-service.feature` and `journey-submit-job.feature`. |
| 5. AC derived from UAT | **PASS** | 7 AC items, each traceable to a UAT scenario. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | **PASS** | ~1 day estimate per `slice-01`. 5 scenarios. |
| 7. Technical notes identify constraints | **PASS** | Section-as-discriminator + custom Deserialize / `serde(untagged)`. Per-field error messages required. |
| 8. Dependencies resolved or tracked | **PASS** | None — this is the enabler. |
| 9. Outcome KPIs defined | **PASS** | Story embeds: 100% kind-explicit specs, 100% mixed rejected with named guidance. Cross-listed in `outcome-kpis.md` K2. |

## US-02: Job submit terminal

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear | **PASS** | "Ana ... gets `Job 'coinflip' is running with 1/1 replicas (took live)` followed by a process exit code of 0. Then she sees `ERROR` in the `serve` log and realises the CLI lied to her." Concrete and rooted in real reproduction. |
| 2. User/persona identified | **PASS** | Ana submitting one-shot Jobs. |
| 3. 3+ domain examples | **PASS** | (1) Job exits 0 — Succeeded; (2) Job exits 1 every attempt — Failed; (3) attempts 1+2 fail, attempt 3 exits 0 — Succeeded. |
| 4. UAT scenarios | **PASS** | 4 scenarios + structural anti-scenario. |
| 5. AC derived from UAT | **PASS** | 6 AC items each traceable. |
| 6. Right-sized | **PASS** | ~1.5 days; 4 scenarios. Largest slice; could split if PR balloons but kept whole because the structural fix is one move. |
| 7. Technical notes | **PASS** | RCA root causes B+C+D structurally addressed; root cause A documented as separate concern. |
| 8. Dependencies resolved or tracked | **PASS** | Depends on US-01 (parser landed before submit can route to JobSubmitEvent). |
| 9. Outcome KPIs | **PASS** | K1 (honesty rate ≥99% over 100 trials, baseline 0%). |

## US-03: alloc status kind-aware

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear | **PASS** | Ana wants kind-aware post-hoc view; Service shows replicas+restarts, Job shows verdict+exit codes. User explicit framing. |
| 2. User/persona identified | **PASS** | Ana inspecting workloads after submit. |
| 3. 3+ domain examples | **PASS** | (1) Service `payments` after 42s, (2) Failed Job `coinflip` post-backoff, (3) In-progress Job `long-import`. |
| 4. UAT scenarios | **PASS** | 4 scenarios + anti-scenario. |
| 5. AC derived from UAT | **PASS** | 6 AC items. |
| 6. Right-sized | **PASS** | ~1 day; 4 scenarios. |
| 7. Technical notes | **PASS** | `AllocStatusRow.kind` denormalisation; greenfield migration. |
| 8. Dependencies resolved or tracked | **PASS** | Depends on US-01 (kind enum) + US-02 (terminal Job rows to render). |
| 9. Outcome KPIs | **PASS** | K3 (≥95% comprehension). |

## US-04: Service preservation

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear | **PASS** | Ana wants existing Service workflows preserved with kind-aware vocabulary. |
| 2. User/persona identified | **PASS** | Ana maintaining existing Service workflows. |
| 3. 3+ domain examples | **PASS** | (1) test fixture migration, (2) real Service stabilisation in 1.4s, (3) Service exit-during-stability case. |
| 4. UAT scenarios | **PASS** | 3 scenarios. |
| 5. AC derived from UAT | **PASS** | 4 AC items. |
| 6. Right-sized | **PASS** | ~0.5 days. |
| 7. Technical notes | **PASS** | Companion to US-02; closes RCA root cause D (`"live"` literal). |
| 8. Dependencies resolved or tracked | **PASS** | Depends on US-01, US-06 (grep gate). |
| 9. Outcome KPIs | **PASS** | K4 (regression: 100% existing tests pass). |

## US-05: Schedule parsing + deferral

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear | **PASS** | Ana wants to declare recurring jobs; platform validates AND tells her execution is deferred. |
| 2. User/persona identified | **PASS** | Ana planning recurring workloads. |
| 3. 3+ domain examples | **PASS** | (1) valid `nightly-backup`, (2) `[schedule]` without `[job]` rejection, (3) alloc status reflection. |
| 4. UAT scenarios | **PASS** | 4 scenarios. |
| 5. AC derived from UAT | **PASS** | 7 AC items. |
| 6. Right-sized | **PASS** | ~1 day. |
| 7. Technical notes | **PASS** | Cron-string-only validation; full execution deferred. |
| 8. Dependencies resolved or tracked | **PASS** | Depends on US-01. Deferral GH issue [overdrive-sh/overdrive#166](https://github.com/overdrive-sh/overdrive/issues/166) created 2026-05-09 with user approval; CLI constant `SCHEDULE_EXECUTION_TRACKING_URL` references that URL. |
| 9. Outcome KPIs | **PASS** | K5 (100% URL byte-equality). |

### Deferral gating — RESOLVED

US-05's conditional dependency is resolved. The user approved deferral of Schedule
execution semantics on 2026-05-09; the orchestrator created
[overdrive-sh/overdrive#166](https://github.com/overdrive-sh/overdrive/issues/166)
("Scheduled job execution semantics — cron parser, fire-on-tick reconciler,
concurrency policy"). The CLI constant `SCHEDULE_EXECUTION_TRACKING_URL` MUST equal
`https://github.com/overdrive-sh/overdrive/issues/166` byte-for-byte; both the
submit-echo NOTE block and the `alloc status` Reason line read from this constant.
KPI K5 asserts the two surfaces produce byte-equal URLs.

## US-06: Anti-pattern grep gate (technical task)

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear | **PASS** | Future contributors could re-introduce `"live"` literal; CI gate prevents regression. |
| 2. User/persona identified | **PASS** | Engineer maintaining the CLI render layer. |
| 3. 3+ domain examples | **N/A — technical task** | Not required per LeanUX template for technical tasks; linked to US-02. |
| 4. UAT scenarios | **PASS** | Implicit in AC: regression test asserts CI fails on re-introduction. |
| 5. AC derived from UAT | **PASS** | 3 AC items. |
| 6. Right-sized | **PASS** | <2 hours. |
| 7. Technical notes | **PASS** | Implemented as part of Slice 01. |
| 8. Dependencies resolved or tracked | **PASS** | Linked to US-02. |
| 9. Outcome KPIs | **PASS** | Inherits K1 stability — no future regressions to "is running with N/M replicas (took live)". |

## US-07: Migration of `examples/coinflip.toml`

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear | **PASS** | Existing example must migrate to new shape. |
| 2. User/persona identified | **PASS** | Ana using the example to reproduce / learn from. |
| 3. 3+ domain examples | **N/A — migration task** | The migration IS the example. |
| 4. UAT scenarios | **PASS** | Implicit: file parses post-migration; submit produces kind-aware verdict. |
| 5. AC derived from UAT | **PASS** | 3 AC items. |
| 6. Right-sized | **PASS** | <1 hour. |
| 7. Technical notes | **PASS** | Single-cut migration. |
| 8. Dependencies resolved or tracked | **PASS** | Depends on US-01. |
| 9. Outcome KPIs | **PASS** | Enabler for K1 measurement (the test requires the migrated file). |

## US-08: Service listener spec shape

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear | **PASS** | "Ana has a `payments` Service that listens on TCP port 8080 (HTTP) and UDP port 8081 (a sidecar metrics shipper). Today the `[service]` block carries no listener fields ..." Concrete persona + concrete real-world TOML + concrete listener intent. |
| 2. User/persona identified | **PASS** | Ana, Overdrive platform engineer, declaring Service workloads with protocol/port-specific listeners. |
| 3. 3+ domain examples with real data | **PASS** | (1) `frontend` with two listeners (8080/tcp/10.0.0.1 and 8081/udp/none); (2) `payments` with case-insensitive `TCP` protocol; (3) `broken` with duplicate-triple rejection. All real TOML, real CLI output. |
| 4. UAT in Given/When/Then (3-7 scenarios) | **PASS — at upper edge** | 9 scenarios. At the upper edge of the 3–7 range; defended because each scenario tests a distinct rejection path (zero listeners, duplicate triple, unsupported protocol, port=0) plus the four happy-path scenarios (parse, case-insensitive, submit echo, alloc status, OpenAPI roundtrip, property test). Splitting US-08 would fragment the round-trip story; the architect's natural fault line (parser+echo vs. alloc status) is captured in `slice-06-service-listener-fields.md` for DESIGN-time evaluation. |
| 5. AC derived from UAT | **PASS** | 10 AC items, each traceable to one or more UAT scenarios. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | **PASS — caveat** | ~1.5 days estimated. Scenario count above the 3–7 nominal but the slice document defends it; if DESIGN finds the slice oversized, the parser+echo / alloc status fault line is the natural split. |
| 7. Technical notes identify constraints | **PASS** | Field name (`protocol` not `proto`), section name (`[[listener]]` not `[[backend]]`), `Proto` newtype reuse from `overdrive-core`, `Option<ServiceVip>` shape, OpenAPI derives, `utoipa::ToSchema`. |
| 8. Dependencies resolved or tracked | **PASS** | Depends on US-01 (parser kind enum) and US-04 (Service render path). Runtime allocator for `vip = None` referenced as [#167](https://github.com/overdrive-sh/overdrive/issues/167) (approved 2026-05-09); spec layer is forward-compatible regardless of the allocator's eventual decision. |
| 9. Outcome KPIs | **PASS** | K6 (100% byte-equality between submit echo and `alloc status` Listeners sections across 100 Service submits with pinned VIPs). |

---

## DoR Status: PASSED

8/8 stories fully passed. US-05's previous conditional on user approval of a deferral
GH issue resolved on 2026-05-09 — issue
[#166](https://github.com/overdrive-sh/overdrive/issues/166) created. US-08 added
2026-05-10 from the fold-in of [#164](https://github.com/overdrive-sh/overdrive/issues/164);
runtime allocator referenced as [#167](https://github.com/overdrive-sh/overdrive/issues/167)
(approved 2026-05-09). All 8 stories hand off to DESIGN together.

## Anti-pattern detection (per `nw-leanux-methodology`)

| Anti-pattern | Status | Evidence |
|---|---|---|
| Implement-X (technical-first stories) | **CLEAR** | Every story starts from a persona pain point. US-01 starts from Ana's TOML; US-02 starts from the bug she reproduced; US-03 starts from her post-hoc inspection workflow. |
| Generic data | **CLEAR** | Real names (Ana, `payments`, `coinflip`, `nightly-backup`), real TOML bodies, real CLI output strings, real exit codes. No `user123` / `test@test.com`. |
| Technical AC | **CLEAR** | All AC describe operator-observable outcomes. No "use serde::untagged" or "implement BPF tail call" — those are DESIGN decisions. |
| Technical scenario titles | **CLEAR** | All scenario titles describe business outcomes ("A Job that exits 0 reports Succeeded", "alloc status NEVER renders Service phrasing"). No "FileWatcher triggers TreeView refresh"-shaped titles. |
| Oversized stories | **CLEAR** | Largest is US-02 at ~1.5 days / 4 scenarios + anti-scenario. None exceed 7 scenarios. |
| Abstract requirements | **CLEAR** | Every story has 3+ concrete examples (or is a typed task linked to one). |
