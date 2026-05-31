# O01 — Job/Schedule + probe is rejected with actionable guidance

**Surface:** O (operator CLI) · **KPI:** K5 · **Status:** `pending`

## Expectation

When an operator submits a **Job-** or **Schedule-**kind spec that carries a
`[[health_check.*]]` section, the CLI rejects it **before** contacting the
control plane, exits non-zero, and the error message **names the kind and
tells the operator what to do instead** — not a cryptic parse error. A
**Service** spec with the same probe section is accepted (regression guard).

This is a qualitative expectation: "the error is actionable, not cryptic."
No `assert!` captures "actionable"; a human reads the evidence and judges.

- Anchor: S-SHCP-PARSE-05 (`ProbesNotAllowedOnKind { kind: "job", guidance: … }`)
- Anchor: S-SHCP-PARSE-06 (`ProbesNotAllowedOnKind { kind: "schedule", guidance: … }`)
- Anchor: S-SHCP-CLI-12,13,14 (CLI-handler boundary: guidance rendered, exit 1, accept is no-op)
- Anchor: docs/feature/service-health-check-probes/discuss/outcome-kpis.md — K5

## Verification

The runner writes three temp specs (job+probe, schedule+probe, service+probe),
runs `overdrive deploy <spec>` for each through Lima, and captures verbatim
stdout/stderr + exit codes. Sub-claims:

1. job+probe → non-zero exit, output names `job` and contains guidance.
2. schedule+probe → non-zero exit, output names `schedule` and contains guidance.
3. service+probe → does **not** fail at the kind gate (accept-case regression
   guard; it may fail later for an unrelated reason like "control plane
   unreachable" — that is a different failure and is recorded, not conflated).

`satisfied` requires: sub-claims 1 & 2 reject with guidance text, and the
guidance is judged actionable by the reviewer (Step 4, adversarial).

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O01`. Not yet run.
