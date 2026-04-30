# Journey (visual) — Submit streams default + alloc status enrichment

**Feature**: `cli-submit-vs-deploy-and-alloc-status`
**Wave**: DISCUSS / Phase 2
**Owner**: Luna (`nw-product-owner`)
**Date**: 2026-04-30
**Type**: Extension of `docs/product/journeys/submit-a-job.yaml`
(further extended by
`docs/feature/phase-1-first-workload/discuss/journey-submit-a-job-extended.yaml`).
This file extends step 1 (Submit) into 1 + 1a + 1b + 1c, and rewrites
step 4 (Inspect) into a richer snapshot.

---

## Scope of this extension

Steps inherited unchanged: 2 (Commit), 3 (Reconciler alive), 5 (Crash
recovery), 6 (Control-plane responsiveness), 7 (Stop). Steps modified:
1 (Submit) — split into 1, 1a, 1b, 1c (intent commit ack, NDJSON
convergence stream, terminal). 4 (Inspect) — enriched snapshot fields.

---

## Persona

Ana — Overdrive platform engineer. Same persona as the base journey.
Senior SRE muscle memory: `kubectl rollout status`, `nomad job run`,
`fly deploy`, `systemctl start && journalctl -fu`. Inner-loop edit-
submit-observe-fix cycle is the dominant case. CI consumption is the
secondary case.

---

## Emotional arc — extended

| Phase | Beat | Feeling | Why |
|---|---|---|---|
| Pre-1 | Edits `payments.toml` after the last failure | Curious / hypothesizing | "Did I fix the binary path?" |
| 1 / 1a (within 200ms) | Sees `Submitted spec sha256:...` | Focused — *not abandoned* | The first NDJSON line lands quickly. The CLI did not hang. |
| 1b (each transition) | Lifecycle events stream inline | Confidence builds *or* failure surfaces | Each event is honest: source, timestamp, structured reason. No fabricated rows. |
| 1c (terminal) | Either "Running 1/1, took 1.4s" + exit 0, or "Error: ... reason: binary not found" + exit 1 | Trusting (success) / informed (failure) | The platform told the truth on the first command. |
| Post-1c | (success) move on; (failure) edit the spec, re-run | Senior-SRE flow re-engaged | Inner loop is one verb, one terminal, one decision. |

The arc must STRENGTHEN trust, not weaken it. Two specific risks:

1. **The 200ms first-event budget is the trust floor.** If `submit`
   appears to hang for 2 seconds before *anything* prints, the
   operator wonders whether the CLI is broken. That is the same
   "abandoned" feeling the user reported when `Accepted.` printed
   for a broken binary — except worse, because now they can't even
   tell if the platform got the request. The 200ms is not an SLO; it
   is an emotional contract.
2. **The convergence stream must end deterministically.** If the
   stream hangs because the reconciler never reports a terminal
   event, the operator has no graceful exit. The server-side wall-
   clock budget (DESIGN-named) is the structural guard.

---

## ASCII flow

```
   ┌──────────────────────────────────────────────────────────────────────┐
   │                                                                      │
   │   $ overdrive job submit ./payments.toml                             │
   │                                                                      │
   └─────────────────────────────────┬────────────────────────────────────┘
                                     │
                                     ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │ Step 1 — Submit (CLI side)                                           │
   │                                                                      │
   │   • CLI reads TOML, constructs typed Job aggregate.                  │
   │   • CLI detects whether stdout is a TTY.                             │
   │       TTY        → Accept: application/x-ndjson                      │
   │       Piped      → Accept: application/json (auto-detach)            │
   │       --detach   → Accept: application/json (explicit detach)        │
   │   • CLI POSTs to /v1/jobs.                                           │
   └─────────────────────────────────┬────────────────────────────────────┘
                                     │
                                     ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │ Step 1a — Intent commit acknowledged (NEW)                           │
   │                                                                      │
   │   First NDJSON line carries the same payload the existing            │
   │   non-streaming response would have returned:                        │
   │     • spec_digest                                                    │
   │     • intent_key                                                     │
   │     • outcome (Inserted / Unchanged per ADR-0020)                    │
   │                                                                      │
   │   AC: this line lands within 200 ms of POST commit. ━━━━━━━━━━━━     │
   │                                                                      │
   │   $ overdrive job submit ./payments.toml                             │
   │   Submitted spec sha256:7f3a... (intent-key payments-v2)             │
   │   ↑                                                                  │
   │   First line. Operator now knows the platform got the request.       │
   └─────────────────────────────────┬────────────────────────────────────┘
                                     │
                                     ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │ Step 1b — Convergence stream (NEW)                                   │
   │                                                                      │
   │   Server subscribes to ObservationStore lifecycle events for the     │
   │   just-committed JobId. Each transition becomes one NDJSON line:     │
   │                                                                      │
   │     payments-v2/a1b2c3   pending     scheduling on local             │
   │     payments-v2/a1b2c3   pending     starting (pid attempt 1)        │
   │     payments-v2/a1b2c3   running     pid 12345, 2000mCPU / 4 GiB     │
   │                                                                      │
   │   Each line carries: alloc_id · from_state · to_state · reason       │
   │   · source (reconciler | driver) · at (timestamp).                   │
   │                                                                      │
   │   Reconciler purity (§18, ADR-0023) preserved: the streaming         │
   │   endpoint is a CONSUMER of observation rows, not a producer that    │
   │   blocks the reconciler tick.                                        │
   └─────────────────────────────────┬────────────────────────────────────┘
                                     │
                                     ▼
   ┌──────────────────────────────────────────────────────────────────────┐
   │ Step 1c — Terminal (NEW)                                             │
   │                                                                      │
   │   Stream closes when convergence reaches one of:                     │
   │     • ConvergedRunning      → CLI prints summary, exits 0.           │
   │     • ConvergedFailed       → CLI prints reason, exits 1.            │
   │     • Server wall-clock cap → CLI prints "did not converge in N s",  │
   │                               exits 1.                               │
   │                                                                      │
   │   Happy path:                                                        │
   │     Job 'payments-v2' is running with 1/1 replicas (took 1.4s).      │
   │     $ echo $?                                                        │
   │     0                                                                │
   │                                                                      │
   │   Failure path (the user's actual session, fixed):                   │
   │     Error: job 'payments-v2' did not converge to running.            │
   │       reason: driver start failed (binary not found)                 │
   │       last-event: stat /usr/local/bin/payments: no such file ...     │
   │       reproducer: overdrive alloc status --job payments-v2           │
   │     $ echo $?                                                        │
   │     1                                                                │
   └─────────────────────────────────┬────────────────────────────────────┘
                                     │
                                     ▼ (post-deployment, Ana types when curious)
   ┌──────────────────────────────────────────────────────────────────────┐
   │ Step 4 — Inspect (REWRITTEN — dense snapshot)                        │
   │                                                                      │
   │   $ overdrive alloc status --job payments-v2                         │
   │                                                                      │
   │   Job:         payments-v2                                           │
   │   Spec digest: sha256:7f3a9b12...                                    │
   │   Replicas:    1 desired / 1 running                                 │
   │                                                                      │
   │   ALLOC ID   STATE      RESOURCES        STARTED              EXIT   │
   │   a1b2c3     Running    2000mCPU/4 GiB   2026-04-30T10:15:32Z  -     │
   │                                                                      │
   │   Last transition: 2026-04-30T10:15:35Z                              │
   │     Pending → Running   reason: driver started (pid 12345)           │
   │     source:  driver(process)                                         │
   │   Restart budget: 0 / 5 used                                         │
   │                                                                      │
   │   ─── Failure path render ───────────────────────────────             │
   │                                                                      │
   │   ALLOC ID   STATE      RESOURCES        STARTED              EXIT   │
   │   a1b2c3     Failed     2000mCPU/4 GiB   2026-04-30T10:15:32Z  -     │
   │                                                                      │
   │   Last transition: 2026-04-30T10:15:35Z                              │
   │     Pending → Failed    reason: driver start failed                  │
   │     source:  driver(process)                                         │
   │     error:   stat /usr/local/bin/payments: no such file or directory │
   │   Restart budget: 5 / 5 used (backoff exhausted)                     │
   │                                                                      │
   │   ─── Replaces ───────────────────────────────────────────            │
   │   The current `Allocations: 1` empty render.                         │
   └──────────────────────────────────────────────────────────────────────┘
```

---

## TUI mockups (canonical)

### Happy path (TTY, default-stream)

```
$ overdrive job submit ./payments.toml
Submitted spec sha256:7f3a9b12... (intent-key payments-v2)
  payments-v2/a1b2c3   pending     scheduling on local
  payments-v2/a1b2c3   pending     starting (pid attempt 1)
  payments-v2/a1b2c3   running     pid 12345, 2000mCPU / 4 GiB

Job 'payments-v2' is running with 1/1 replicas (took 1.4s).
$ echo $?
0
```

### Failure path — binary not found (TTY, default-stream)

```
$ overdrive job submit ./payments.toml
Submitted spec sha256:7f3a9b12... (intent-key payments-v2)
  payments-v2/a1b2c3   pending     scheduling on local
  payments-v2/a1b2c3   pending     starting (pid attempt 1)
  payments-v2/a1b2c3   failed      driver: stat /usr/local/bin/payments: no such file or directory
  payments-v2/a1b2c3   pending     starting (pid attempt 2; backoff 5s)
  payments-v2/a1b2c3   failed      driver: stat /usr/local/bin/payments: no such file or directory
  ...

Error: job 'payments-v2' did not converge to running.
  reason: driver start failed (binary not found)
  last-event: stat /usr/local/bin/payments: no such file or directory
  reproducer: overdrive alloc status --job payments-v2

Hint: fix the spec's `exec.command` path and re-run.
$ echo $?
1
```

### `--detach` (CI / automation)

```
$ overdrive job submit ./payments.toml --detach
{"spec_digest":"sha256:7f3a9b12...","intent_key":"payments-v2","outcome":"Inserted"}
$ echo $?
0
```

### Pipe auto-detach (interactive but non-TTY)

```
$ overdrive job submit ./payments.toml | jq -r .spec_digest
sha256:7f3a9b12...
$ echo $?
0
```

### `alloc status` enriched — Running case

```
$ overdrive alloc status --job payments-v2
Job:         payments-v2
Spec digest: sha256:7f3a9b12...
Replicas:    1 desired / 1 running

ALLOC ID   STATE      RESOURCES        STARTED               EXIT
a1b2c3     Running    2000mCPU/4 GiB   2026-04-30T10:15:32Z  -

Last transition: 2026-04-30T10:15:35Z
  Pending → Running   reason: driver started (pid 12345)
  source:  driver(process)
Restart budget: 0 / 5 used
```

### `alloc status` enriched — Failed case

```
$ overdrive alloc status --job payments-v2
Job:         payments-v2
Spec digest: sha256:7f3a9b12...
Replicas:    1 desired / 0 running

ALLOC ID   STATE      RESOURCES        STARTED               EXIT
a1b2c3     Failed     2000mCPU/4 GiB   2026-04-30T10:15:32Z  -

Last transition: 2026-04-30T10:15:35Z
  Pending → Failed    reason: driver start failed
  source:  driver(process)
  error:   stat /usr/local/bin/payments: no such file or directory
Restart budget: 5 / 5 used (backoff exhausted)
```

---

## Shared artifacts crossing the journey

See `shared-artifacts-registry.md` for the full table. Highlights:

- `spec_digest` — inherited unchanged; appears in step 1a and step 4.
- `intent_key` — inherited unchanged; appears in step 1a.
- `convergence_event` (NEW) — single typed enum in
  `overdrive-control-plane::api`; consumed by step 1b's stream
  renderer.
- `alloc_status_snapshot` (NEW or extended) — single typed struct in
  `overdrive-control-plane::api`; consumed by step 4 renderer.
- `failure_reason` — same source (lifecycle reconciler view +
  driver-error pass-through) used by both the streaming submit
  terminal event AND the alloc-status snapshot. Two consumption
  surfaces, one source of truth.

---

## Failure modes covered

| Failure | Stream rendering | Snapshot rendering | Exit code |
|---|---|---|---|
| Binary not found | `failed driver: stat ...` lines until backoff exhausted, then terminal failure | `Failed` + `error:` line + `backoff exhausted` | 1 |
| Capacity exceeded (resources too high for the local node) | `pending: no node has capacity (...)` until wall-clock cap | `Pending` + reason naming requested-vs-free | 1 (cap hit) |
| Network failure to control plane (server down) | No NDJSON; CLI prints transport error | n/a | 2 |
| Bad TOML (client-side validation) | No POST happens; CLI prints validation error | n/a | 2 |
| Server returns 400 (validation failure on aggregate) | No NDJSON stream; single JSON `ErrorBody` per ADR-0015 | n/a | 2 |
| Server-side wall-clock cap hit | Stream closes with `ConvergedFailed` carrying `terminal_reason: timeout` | n/a (snapshot still shows `Pending`) | 1 |

---

## Out of scope (will not land in this feature)

- `alloc status --follow` / `--watch` — not in this feature; may
  arrive later as a separate divergence.
- Multi-replica progress rendering (the journey is `replicas = 1` per
  the Phase 1 single-node constraint).
- TUI-mode (ratatui) rendering — same NDJSON stream forward-compatibly
  consumed; out of scope for Phase 1.
- Any change to `cluster status`, `logs`, or the existing OpenAPI
  schema's discoverability surface.
- Any change to `job stop`'s shape — already shipped in
  phase-1-first-workload.

---

## Open question carried into DESIGN

**The server-side wall-clock cap value** is a DESIGN call. DISCUSS
states it must exist (because operators must not stare at a stuck
terminal forever) and recommends 60 seconds as a starting point
informed by the lifecycle reconciler's typical backoff behaviour. The
ADR formalises the value and the surface (CLI flag override?
config?). DISCUSS does not block on this.
