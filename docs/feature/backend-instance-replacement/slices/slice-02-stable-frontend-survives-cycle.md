# Slice 02 — The stable address binding survives the cycle (no stale address)

> DISCUSS brief (2026-06-29). Feature: `backend-instance-replacement` (#249). Story: **US-BIR-2** (stable-`F` half).
> Job: J-OPS-003 (extended). Builds on slice-01. Effort: ≈0.5d.

## Goal (one line)

After the replace action, `getaddrinfo("<job>.svc.overdrive.local")` re-resolves to the
**same byte-identical `F`** and the next connect lands the **NEW** backend instance —
no stale address (the SQ1-elimination guarantee).

## Learning hypothesis

The `FrontendAddrAllocator`'s idempotent `assign("<job>")` (withhold-not-release;
per-logical-workload) retains `F` across the cycle, and the re-keyed `MtlsResolve`
re-resolves the live backend per-connect, so a peer re-resolving the name lands the
fresh instance through the same `F`.
**Predicted:** `f1_again == f1` byte-for-byte; the post-cycle connect to `F1` lands `B2`
byte-exact; `F1 ∈ 10.98.0.0/16` is never a backend addr `∈ 10.99.0.0/16`.

## Thinnest serve+deploy loop

`overdrive serve` + deploy `server` (Running behind `F1`, backend `B1`) + a deployed
client resolves `server.svc.overdrive.local` → `F1`, connect lands `B1` (byte-exact) +
`overdrive workload restart server` (→ `B2`) + client re-resolves → SAME `F1`, connect lands `B2`.

## Behavior (implemented per ADR-0073)

- `overdrive workload restart <id>` (the slice-01 verb) retains the per-logical-workload `F`-binding across the cycle (no churn, no release).
- The re-keyed `MtlsResolve` translates `F` → the NEW live backend per-connect.
- Single-source: the resolved `F` is the addr `MtlsResolve.resolve` recognizes and classifies `Mesh`.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — the resolve + dial run from a deployed workload's production netns through the production responder + intercept; the byte-exact round-trip to `B2` is the proof.
- **Thinnest?** Yes — the next-dial-is-live half only (in-flight churn fail-fast is slice-03).
- **No `#[test]`-only composition?** Driven through the production responder + `MtlsResolve` + intercept, not a hand-rolled resolver.

## Acceptance (= US-BIR-2 stable-`F` ACs)

- [ ] After the replace action, `getaddrinfo("<job>.svc.overdrive.local")` re-resolves to the SAME `F` byte-for-byte.
- [ ] The next connect to that `F` lands the NEW backend `B2` (byte-exact round-trip to the fresh instance).
- [ ] The resolved value is always `F ∈ 10.98.0.0/16`, never a backend addr `∈ 10.99.0.0/16` (neither `B1` nor `B2`).
- [ ] Proven through `serve` + `deploy` + the replace action (Tier-3), consistent with the dial-by-name intercept path — no second source of backend truth.

## Dependencies

- **slice-01** (the `overdrive workload restart <id>` verb exists, per ADR-0073).
- SHIPPED: dial-by-name responder + `FrontendAddrAllocator` idempotent `assign` + re-keyed `MtlsResolve` + intercept path (#243 / #26 / #236).
