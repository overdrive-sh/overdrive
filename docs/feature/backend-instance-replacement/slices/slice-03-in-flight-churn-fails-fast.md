# Slice 03 — In-flight connection fails fast on backend churn

> DISCUSS brief (2026-06-29). Feature: `backend-instance-replacement` (#249). Story: **US-BIR-2** (churn-boundary half).
> Job: J-OPS-003 (extended). Builds on slice-01/02. Effort: ≈0.5d.

## Goal (one line)

A client holding an open in-flight connection to the current backend, when that backend
is cycled mid-connection, gets a PROMPT reset/error bounded by `TCP_USER_TIMEOUT` (never
an indefinite hang), and a subsequent fresh connect lands the new live backend.

## Learning hypothesis

The terminating-proxy posture (`TCP_USER_TIMEOUT`/keepalive on the worker proxy legs,
`mtls_intercept_worker.rs`) surfaces backend death to the in-flight connection promptly —
WITHOUT `sock_destroy` (#61 scope) — and the next fresh dial re-resolves the live backend.
**Predicted:** the in-flight read returns (reset/error/EOF) within `CHURN_BOUND`; the
subsequent fresh connect to `F` lands `B2` byte-exact.

## Thinnest serve+deploy loop

`overdrive serve` + deploy `server` (Running, `B1`) + a deployed client opens an in-flight
connection through the intercept to `B1` (data flowing) + the replace action on `server`
mid-connection + the in-flight read returns promptly + a subsequent fresh connect lands `B2`.

## Behavior (DESIGN owns API)

- Cycling the backend mid-connection causes the in-flight connection to fail fast (the pump task + `TCP_USER_TIMEOUT`/keepalive on the worker proxy legs), bounded — NOT an indefinite hang, NO `sock_destroy`.
- A subsequent fresh connect to `F` re-resolves and lands the new live backend `B2`.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — the in-flight connection + the churn + the fresh dial all run through the production intercept + the replace action; the bounded-elapsed measurement is the proof.
- **Thinnest?** Yes — the fail-fast + next-dial-live behavior only (the stable-`F` byte-equality is slice-02).
- **No `#[test]`-only composition?** Driven through the production intercept worker + the replace action.

## Acceptance (= US-BIR-2 churn ACs)

- [ ] An in-flight connection to the old instance, when its backend is cycled mid-connection, fails fast (reset/error/EOF) bounded by `TCP_USER_TIMEOUT` — never an indefinite hang; NO `sock_destroy`.
- [ ] A subsequent fresh connect to `F` lands the new live backend `B2` (byte-exact round-trip).
- [ ] Proven through `serve` + `deploy` + the replace action (Tier-3), consistent with the dial-by-name intercept path.

## Dependencies

- **slice-01/02** (the replace action + stable `F`).
- SHIPPED: the intercept worker `TCP_USER_TIMEOUT`/keepalive legs (`mtls_intercept_worker.rs`).
