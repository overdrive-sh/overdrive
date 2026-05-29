# Expectations — master status table

Surfaces: **O** operator CLI · **R** reconciler/convergence · **D** dataplane/kernel · **E** end-to-end · **X** build/supply-chain.
Status: `pending | satisfied | partial | broken | unanchored-claim | out-of-scope` (see `../README.md`).

| ID | Surface | Expectation | KPI | Anchors | Status |
|---|---|---|---|---|---|
| [O01](O01-kind-rejection-guidance/) | O | Job/Schedule + probe rejected with actionable guidance | K5 | S-SHCP-PARSE-05/06, CLI-12..14 | `pending` |
| [O02](O02-alloc-status-probes-section/) | O | `alloc status` renders a Probes section for a Service | K4 | S-SHCP-CLI-01..06 | `pending` |
| [E01](E01-coinflip-service-honest-early-exit/) | E | coinflip-as-Service honest EarlyExit, never `(took live)` | K1 | S-SHCP-RECON-04, INT-CLI-01, CLI-07..11 | `pending` |

## Feature coverage

- **service-health-check-probes** — O01, O02, E01 (operator + e2e surfaces).
  The in-process behaviour is covered by the four test tiers; these capture
  the operator-observable and qualitative slice those tiers under-serve.

## Adding an expectation

1. `mkdir verification/expectations/<SURFACE><NN>-<slug>/` with a `README.md`
   (scenario + `- Anchor:` lines + verification block + `Status: pending`).
2. Add an optional `runner.sh` that drives the **built** `overdrive` binary
   via the `od` helper (real commands; executed in Lima).
3. Add a row here.
4. Run `harness/run-expectation.sh <ID>`, review the evidence adversarially,
   then set the status in the expectation's `README.md`.
