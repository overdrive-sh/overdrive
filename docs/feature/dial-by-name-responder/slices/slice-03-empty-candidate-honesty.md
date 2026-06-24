# Slice 03 — Empty-candidate honesty

> Reviewed brief (DISCUSS, 2026-06-24; gated to Slice 00). Feature: `dial-by-name-responder` (#243). Story: **US-DBN-4**.
> Job: J-MESH-001. The fail-honest leg. Builds on Slice 01.

## Goal (one line)

When a queried name has **no running-AND-healthy backend**, the responder returns
**NXDOMAIN** (per the v1 DNS answer contract) — **never** a stale, cached, unhealthy,
or last-known address.

## Learning hypothesis

The `running-AND-healthy` filter on the name index (the `by_name` index gates
`Backend.healthy == true`, over the SAME `service_backends` rows the intercept reads)
is the single source of liveness truth for the name layer, so an empty
`service_backends ∩ running-and-healthy` set produces **NXDOMAIN** — matching the
arc's fail-closed/fail-honest discipline. **A stale or unhealthy address is worse than
no address** (it sends an unmodified workload to a dead or not-ready instance — which
the intercept would anyway classify `MeshUnreachable`).

## serve+deploy loop

`overdrive serve` + `overdrive deploy server.toml`, then query
`server.svc.overdrive.local` **before** it reaches running-and-healthy (and after
`overdrive job stop`) → NXDOMAIN. The observable is the query result inside a
deployed workload + the absence of a bogus connection. No new operator verb.

## Behavior

- Empty `running-AND-healthy` candidate set for the queried name → **NXDOMAIN** (declared-but-not-running, unhealthy, and unknown all collapse in v1 — the responder reads only the running-and-healthy index; **NODATA** is reserved for `AAAA` on a name that IS resolvable, not the empty-candidate case).
- After all backends stop (or go unhealthy) → the name stops resolving (no stale addr).
- Unknown name → NXDOMAIN.
- Mirrors "never absorb a fallible read into a default" + the K8s-headless / Fly `.internal` empty-endpoint-set shape.

## Carpaccio taste tests

- **Closes a real loop through production?** Yes — proven by a deployed workload's query against `serve` + `deploy` (Tier-3), consistent with the index's `running-and-healthy` filter. No second source of liveness truth.
- **Thinnest?** Yes — one behavior (no-running-and-healthy-backend honesty → NXDOMAIN), boundary inputs across not-running / unhealthy / unknown.
- **No `#[test]`-only composition?** The query runs from inside a deployed workload against the production responder.

## Acceptance (= US-DBN-4 ACs)

- [ ] Empty `running-and-healthy` set → **NXDOMAIN** (never stale/cached/unhealthy/guessed).
- [ ] After all backends stop (or go unhealthy), the name stops resolving (no stale addr).
- [ ] Unknown name → NXDOMAIN.
- [ ] Proven Tier-3 through a deployed workload's query against `serve` + `deploy`; consistent with the index's `running-and-healthy` filter.

## Dependencies

- **Slice 01** (the resolve path + the `running-and-healthy`-filtered name index).
- Consistency with the arc's `running-and-healthy` liveness definition (no new liveness source; an unhealthy backend is `MeshUnreachable` to the intercept, so it must not be answered).
