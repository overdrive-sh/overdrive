# Slice 07 — Reject probes on Job/Schedule with named guidance

**Stories:** US-07
**Priority:** P1
**KPI:** K5 (Misshapen-spec named-error rate: 100% of `[job]`/`[schedule]` + `[[health_check.*]]` specs reject at parse time with kind-specific guidance)
**Dependencies:** ADR-0047 (landed) — independent of Slices 01–06; can land in parallel with Slice 01

## Outcome the operator can verify

Operator drops `[[health_check.startup]]` under `[job]`:

```
Error: probes not allowed on workload kind 'job'

  Job has no readiness question; on completion is enough.
  Use exit code 0 to indicate success.

  Remove the [[health_check.*]] sections from your spec, or change
  [job] to [service] if this workload is intended to be long-lived.
```

Exit code 1; no IntentStore write.

## Adds

| Component | Change |
|---|---|
| TOML parser | New `ParseError::ProbesNotAllowedOnKind { kind, guidance }` variant |
| Validation pass | After kind detection per ADR-0047, scan for `[[health_check.*]]` arrays under non-Service blocks |
| CLI error rendering | Per-kind guidance text (constants in `overdrive-core::parse_error`) |

## Acceptance test additions

- Job + startup probe → reject with job-specific guidance
- Job + readiness probe → reject (same)
- Job + liveness probe → reject (same)
- Schedule + any probe → reject with schedule-specific guidance
- Service + probes → no error (regression test)

## Demoable check

`cargo xtask lima run -- cargo nextest run -p overdrive-core -E 'test(parse_error_probes_not_allowed)'` passes.

## Independence

Pure parser change. Can land before, in parallel with, or after Slice 01.
