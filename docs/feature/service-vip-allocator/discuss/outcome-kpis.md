# Outcome KPIs — service-vip-allocator

## Feature: service-vip-allocator

### Objective

Operators submitting Service specs trust the platform to issue dataplane
VIPs transparently, idempotently, and without operator-visible pool
management. The allocator never silently leaks, never silently
duplicates, and never silently exhausts.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| K1 | Operators submitting Service specs without operator-supplied `vip` against a non-empty pool | Successfully submit and receive an allocated VIP | 100% success rate (no silent allocation failures) | 0% (pre-feature, operators cannot submit without a VIP at all) | Allocator successful-allocation rate = `admissions_with_allocated_vip / admissions_attempted_without_operator_vip_against_nonempty_pool` | Leading (Outcome) |
| K2 | Operators submitting Service specs against a single-node control plane | Experience allocator-induced submit latency below operator-noticeable threshold | p50 ≤ 5 ms, p99 ≤ 25 ms allocator-attributable latency added to admission | N/A (new code path; baseline established at first measurement) | Allocator-induced admission latency, isolated from total admission latency via per-stage instrumentation | Leading (Secondary) |
| K3 | The platform | Releases allocated VIPs promptly on terminal-state transition | p50 ≤ 1 s, p99 ≤ 5 s lag from terminal-state observation to VIP release | N/A (new code path) | VIP reclamation lag = `release_timestamp - terminal_state_observation_timestamp` | Leading (Secondary, Guardrail) |
| K4 | The allocator pool | Stays out of exhaustion under normal load | 0 pool-exhaustion rejections per 24-hour window under nominal workload churn (≤ 50% of pool capacity at steady state) | N/A | Pool exhaustion rejection count over rolling 24-hour window; pool utilisation % at steady state | Leading (Guardrail) |

### Metric Hierarchy

- **North Star**: K1 (successful-allocation rate) — the single
  metric that captures whether the feature delivers on its core
  promise.
- **Leading Indicators**: K2 (latency) predicts operator perception;
  K3 (reclamation lag) predicts whether the pool drains correctly
  over time.
- **Guardrail Metrics**: K3 and K4 — these must NOT degrade. A high
  K3 means leaks accumulate and K4 will eventually trip. A non-zero
  K4 under nominal load means either K3 is broken or the pool is
  sized wrong.

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|-----|------------|-------------------|-----------|-------|
| K1 | Control-plane admission audit log + allocator state | Counter increments at admission entry / allocator allocate / admission success; ratio computed at scrape time | Per-admission event; aggregated per-minute | DEVOPS (platform-architect) |
| K2 | Per-stage admission span timings | Span start/end at allocator entry / exit; histogram bucketed | Per-admission event; p50/p99 computed over rolling 5-minute window | DEVOPS |
| K3 | Reconciler observation events (terminal-state transition) + allocator release events | Two timestamps, difference computed; histogram bucketed | Per-terminal-state event; p50/p99 computed over rolling 1-hour window | DEVOPS |
| K4 | Allocator state snapshot + admission rejection counter (typed as `pool_exhausted`) | Counter increment on rejection; gauge for utilisation % | Per-rejection event; gauge sampled per-minute | DEVOPS |

### Hypothesis

We believe that a platform-issued Service VIP allocator (DISCUSS-scope
primitive in `crates/overdrive-dataplane/`) for Overdrive operators
will achieve:

- 100% successful-allocation rate against a non-empty pool (K1)
- ≤ 5 ms p50 / 25 ms p99 allocator-induced admission latency (K2)
- ≤ 1 s p50 / 5 s p99 VIP reclamation lag (K3)
- 0 pool-exhaustion rejections per 24-hour window under nominal load
  (K4)

We will know this is true when, in a Phase 1 single-node deployment
running 30+ days of mixed workload churn, K1 stays at 100%, K2 stays
below thresholds, K3 stays below thresholds, and K4 stays at zero —
across every single observed admission attempt.

### Smell Test Verification

| Check | Question | Verdict |
|-------|----------|---------|
| Measurable today? | Can K1–K4 be measured with current instrumentation? | NO — DEVOPS must add instrumentation for each. This is a known handoff item, not a blocker for DISCUSS. |
| Rate not total? | Is each KPI a ratio/rate, not a gross count? | YES — K1 is a rate; K2/K3 are latency distributions; K4 has both a rate (rejections per window) and a gauge (utilisation %). |
| Outcome not output? | Does each describe user behavior or platform behavior the user observes, not feature delivery? | YES — none of K1–K4 measure "did we ship the allocator"; each measures observable platform behavior under operator use. |
| Has baseline? | Do we know the current value? | K1 has baseline 0% (pre-feature impossibility). K2/K3/K4 have no baseline (new code path); first measurement establishes baseline. |
| Team can influence? | Can the team directly affect this metric? | YES — every KPI is a function of allocator + reconciler code in this repo. |
| Has guardrails? | Are there metrics that must not degrade? | YES — K3 and K4 are explicit guardrails. |

### Handoff to DEVOPS

The platform-architect needs these from this document to plan
instrumentation:

1. **Data collection requirements**:
   - Counter on admission entry (gated on "no operator-supplied vip");
     counter on allocator-allocated VIP; counter on admission success
     (K1 numerator + denominator).
   - Span timing on allocator entry / exit, isolated from other
     admission stages (K2).
   - Two timestamps per allocation lifecycle: terminal-state observation
     and release acknowledgment (K3).
   - Counter on typed `pool_exhausted` rejection; gauge on pool
     utilisation % (K4).
2. **Dashboard / monitoring needs**:
   - K1 displayed as a single-pane rate (% over rolling 5-min window).
   - K2 as a latency histogram with p50 / p99 callouts.
   - K3 as a latency histogram with p50 / p99 callouts.
   - K4 dual-display: rejection count rate + utilisation % gauge.
3. **Alerting thresholds**:
   - K1 < 99.9% over 15 minutes → page (correctness regression).
   - K2 p99 > 25 ms over 15 minutes → warn.
   - K3 p99 > 5 s over 15 minutes → warn (leak suspicion).
   - K4 utilisation > 80% → warn; > 95% → page (exhaustion imminent).
4. **Baseline measurement**:
   - K1: zero pre-feature (no allocator existed); first observation
     at feature ship time.
   - K2 / K3 / K4: establish baseline at feature ship time; revisit
     thresholds after 30 days of production data.

### Cross-references

- `wave-decisions.md` § Open questions for DESIGN — open questions 1
  and 2 (reclamation trigger; when admission allocates) drive K3 and
  K2 respectively.
- `user-stories.md` § US-01 → Outcome KPIs — story-level summary
  of K1–K4.
- SSOT: [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).
