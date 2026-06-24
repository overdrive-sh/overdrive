# Slice 03 — Empty-candidate honesty

> Reviewed brief (DISCUSS, 2026-06-24; gated to Slice 00). Feature: `dial-by-name-responder` (#243). Story: **US-DBN-4**.
> Job: J-MESH-001. The fail-honest leg. Builds on Slice 01.

## Goal (one line)

When a queried name has **no running backend**, the responder returns **NXDOMAIN**
(per the v1 DNS answer contract) — **never** a stale, cached, or last-known address.

## Learning hypothesis

The `running` filter on the shared resolve index is the single source of liveness
truth for the name layer, so an empty `service_backends ∩ running` set produces
**NXDOMAIN** — matching the arc's fail-closed/fail-honest discipline. **A
stale address is worse than no address** (it sends an unmodified workload to a dead
instance).

## serve+deploy loop

`overdrive serve` + `overdrive deploy server.toml`, then query
`server.svc.overdrive.local` **before** it reaches Running (and after
`overdrive job stop`) → NXDOMAIN. The observable is the query result inside a
deployed workload + the absence of a bogus connection. No new operator verb.

## Behavior

- Empty `running` candidate set for the queried name → **NXDOMAIN** (declared-but-empty and unknown both collapse in v1 — the responder reads only the running index; **NODATA** is reserved for `AAAA` on a name that IS resolvable, not the empty-candidate case).
- After all backends stop → the name stops resolving (no stale addr).
- Unknown name → NXDOMAIN.
- Mirrors "never absorb a fallible read into a default" + the K8s-headless / Fly `.internal` empty-endpoint-set shape.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — proven by a deployed workload's query against `serve` + `deploy` (Tier-3), consistent with the resolve-index `running` filter. No second source of liveness truth.
- **Thinnest?** Yes — one behavior (no-running-backend honesty → NXDOMAIN), three boundary inputs.
- **No `#[test]`-only composition?** The query runs from inside a deployed workload against the production responder.

## Acceptance (= US-DBN-4 ACs)

- [ ] Empty `running` set → **NXDOMAIN** (never stale/cached/guessed).
- [ ] After all backends stop, the name stops resolving (no stale addr).
- [ ] Unknown name → NXDOMAIN.
- [ ] Proven Tier-3 through a deployed workload's query against `serve` + `deploy`; consistent with the resolve-index `running` filter.

## Dependencies

- **Slice 01** (the resolve path + the shared `running`-filtered index).
- Consistency with the arc's `running` liveness definition (no new liveness source).
