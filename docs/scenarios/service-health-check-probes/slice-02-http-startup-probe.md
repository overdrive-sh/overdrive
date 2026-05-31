# Slice 02 — Explicit HTTP startup probe

**Stories:** US-02
**Priority:** P1
**KPI:** K1
**Dependencies:** Slice 01

## Outcome the operator can verify

Ana declares `[[health_check.startup]] type = "http", path = "/healthz", port = 8080` in her TOML. Workload returns HTTP 503 for 8s then 200. CLI prints `Service 'payments' is stable\n  settled_in: 10.1s\n  witness: startup probe #0 (http GET http://0.0.0.0:8080/healthz)`.

## Adds onto Slice 01

| Component | Change |
|---|---|
| TOML parser | Accept `type = "http", path, port, [timeout_seconds], [interval_seconds], [max_attempts]` |
| `ProbeRunner` | New `HttpProbe` dispatcher (plain HTTP only — C6) |
| `ParseError::HttpProbeMissingPath { probe_idx }` | New variant |
| CLI render | `witness` line formats HTTP probe as `http GET http://<host>:<port><path>` |

## Acceptance test additions

- HTTP probe with 2xx response → Pass
- HTTP probe with persistent 503 → Failed with `last_fail: "HTTP 503"`
- HTTP probe with `connection refused` → captured as named last_fail
- Missing `path` → `ParseError::HttpProbeMissingPath { probe_idx: 0 }`
- HTTP 3xx (redirect) response → Fail; probe does NOT follow redirects (research § 6.1 Pitfall 5; see US-02 AC)
- HTTP method is GET only; request has no body (Phase 1; see US-02 AC + Technical Notes)

## Demoable check

`cargo xtask lima run -- cargo nextest run -p overdrive-cli --features integration-tests -E 'test(http_startup_probe)'` passes.

## Out of scope

HTTPS / mTLS / gRPC / retries-with-backoff-inside-an-attempt (deferred to Phase 3+).
POST and custom HTTP methods (deferred to Phase 2).
Redirect-following (3xx treated as Fail; no follow).
