# E01 — coinflip-as-Service reports an honest EarlyExit, never "(took live)"

**Surface:** E (end-to-end) · **KPI:** K1 (closes RCA-A) · **Status:** `pending`

## Expectation

A Service whose process exits 1 within the startup deadline (the
`coinflip-as-service.toml` fixture) deployed end-to-end through a real
control plane produces a terminal **`Failed { EarlyExit { exit_code: 1 } }`**
streaming event — and the rendered operator output **never** contains the
literal `"(took live)"`. The early-exit render is honest: it shows exit code,
elapsed, and a stderr tail; it does not claim the workload became live.

The K1 north-star is statistical (≥99/100 honest over deterministic seeds);
that lives as the Rust regression test `S-SHCP-INT-CLI-01`. This expectation
is the **end-to-end operator-surface proof** for a single pinned seed — the
human-readable companion to that test.

- Anchor: S-SHCP-RECON-04 (`EarlyExit` emitted when alloc exits before startup Pass)
- Anchor: S-SHCP-INT-CLI-01 (coinflip-as-Service 100-seed K1 regression; this is its e2e witness)
- Anchor: S-SHCP-CLI-07..11 (`Failed { EarlyExit }` render; never `"(took live)"`)
- Anchor: US-08; docs/feature/service-health-check-probes/discuss/outcome-kpis.md — K1

## Verification

Precondition: a control plane is reachable (the runner checks; if not, it
prints the exact `overdrive serve` command and exits `pending` rather than
guessing a background-process lifecycle — leaked cgroups/XDP are a documented
hazard, see `.claude/rules/testing.md`).

The runner deploys `crates/overdrive-cli/examples/coinflip-as-service.toml`
through Lima and captures the streaming output verbatim. Sub-claims:

1. Output contains a terminal `Failed` / `EarlyExit` with `exit_code: 1`.
2. Output **never** contains `"(took live)"` (RCA-A guard).
3. Output never contains a `Stable` terminal event.

`satisfied` requires all three, on a Lima run, with the seed pinned in
`evidence/verification.yaml`.

## Evidence

Captured under `evidence/` by `harness/run-expectation.sh E01`. Not yet run.
