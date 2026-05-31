# O02 — `overdrive alloc status` renders a Probes section for a Service

**Surface:** O (operator CLI) · **KPI:** K4 · **Status:** `pending`

## Expectation

For a **Service** alloc with health-check probes, `overdrive alloc status
<job>` renders a **Probes** section: one row per probe, each showing role
(startup / readiness / liveness), probe index, a mechanic summary (e.g.
`tcp 0.0.0.0:8080`), last status, and last-observed timestamp. A probe with
no result yet renders `pending` (not blank); an inferred default probe
renders an `(inferred)` suffix. A **Job-** or **Schedule-**kind alloc renders
**no** Probes section at all.

- Anchor: S-SHCP-CLI-01..06 (Probes-section render contracts)
- Anchor: docs/feature/service-health-check-probes/discuss/outcome-kpis.md — K4

## Verification

Precondition: a control plane is reachable and a Service has been deployed
(the runner uses `crates/overdrive-cli/examples/quick-bind-service.toml`,
which binds and reaches Stable). If the control plane is unreachable the
runner prints the `overdrive serve` + `overdrive deploy` commands and exits
`pending`.

The runner deploys the quick-bind Service, then runs
`overdrive alloc status <job>` through Lima and captures the render verbatim.
Sub-claims:

1. The render contains a `Probes` heading / section.
2. At least one probe row shows a role + mechanic summary (e.g. `tcp `).
3. A probe with no result renders `pending`, not a blank cell.
4. (Negative) Deploying `examples/coinflip.toml` (Job kind) and rendering its
   status shows **no** Probes section.

`satisfied` requires sub-claims 1–4 on a Lima run, reviewed adversarially for
"is the row actually legible to an operator?" (Step 4 — don't outsource taste).

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh O02`. Not yet run.
