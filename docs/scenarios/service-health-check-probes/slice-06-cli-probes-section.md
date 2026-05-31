# Slice 06 — `alloc status` Probes section

**Stories:** US-06
**Priority:** P3
**KPI:** K4 (Operator probe-visibility coverage: 100% of Service allocs with probes render section; 0% of Job/Schedule)
**Dependencies:** Slices 01–05 (renders all mechanic types and roles coherently)

## Outcome the operator can verify

```
$ overdrive alloc status --job payments

Service: payments
  spec_digest:  sha256:abcd…
  replicas:     1/1 stable
  stable_since: 2026-05-24T18:42:13Z

Allocations:
  alloc-payments-0   state=Running   terminal=Stable
    Probes:
      startup   #0  tcp 0.0.0.0:8080         last=ok    at 18:42:11Z  (inferred)
      readiness #0  http GET /healthz        last=ok    at 18:42:43Z
      liveness  #0  http GET /healthz        last=fail  at 18:42:43Z  HTTP 503  (2/3)
```

## Adds onto Slices 01–05

Pure render-layer change.

| Component | Change |
|---|---|
| `crates/overdrive-cli/src/render.rs` | New `probes_section` function; per-mechanic summary formatter |
| Renderer guard | Section iff `kind == Service AND probes_present` |
| Snapshot tests | `crates/overdrive-cli/tests/integration/render_probes_section.rs` using `insta` |
| `(inferred)` marker | Rendered for the default probe when no explicit declaration |

## Acceptance test additions

- Service with probes → section present (snapshot)
- Job with terminal Completed → section absent (snapshot)
- Schedule registered → section absent (snapshot)
- Just-spawned Service alloc (no ProbeResultRow yet) → renders `last=pending`
- NO_COLOR env var respected (acceptance test)

## Demoable check

Snapshot tests pass; manual `overdrive alloc status --job <any-service>` shows readable Probes section.

## Out of scope

JSON output format for Probes section (a future `--json` flag enhancement; not in this slice).
