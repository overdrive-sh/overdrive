# Journey Visual — Service Honest Stable

**Goal:** An operator submits a Service-kind workload through `overdrive job submit <spec>` and receives a streaming wire signal that reflects the Service's **operator-meaningful liveness**, not the kernel's bare-fork acceptance.

**Persona:** Ana — Overdrive platform engineer, single-node dev host, has been bitten by the coinflip exit-1 RCA. Wants the CLI to tell her the truth.

**Secondary persona:** Omar — operator who will use the same CLI once auth lands (Phase 5).

## Emotional arc

| Phase | State | Why |
|---|---|---|
| Start | **Anxious** | "Last time the CLI said 'is running' for a coinflip that exited 1. Will it lie again?" |
| Submit | **Skeptical** | The 200 OK + `Accepted` line lands, but the operator does not yet trust the next line. |
| Probe wait | **Focused** | The stream is open; CLI prints progress (`Probing startup [http://127.0.0.1:8080/healthz] attempt 3/N…`) — the operator can see work happening. |
| Stable | **Trusting** | `Service 'payments' is stable (took 1.2s, startup probe http://127.0.0.1:8080/healthz)` — the literal `took` is a real duration; the witness names the probe that decided. |
| OR Failed | **Relieved (counter-intuitive)** | `Service 'payments' failed: startup probe timed out after 60s (TCP 127.0.0.1:8080 connection refused on every attempt)` — the CLI is HONEST. Operator can now act. |

## Happy path ASCII flow

```
[Ana]                       [CLI overdrive]           [server: control-plane]      [worker]                [obs store]
  |                              |                              |                       |                       |
  | submit payments.toml         |                              |                       |                       |
  |----------------------------->|                              |                       |                       |
  |                              | POST /v1/submit              |                       |                       |
  |                              |----------------------------->|                       |                       |
  |                              |                              | validate spec         |                       |
  |                              |                              | parse [[health_check.startup]]                |
  |                              |                              | OR infer default TCP probe (no decls)         |
  |                              |                              | IntentStore::put                              |
  |                              |   200 OK + Accepted          |                       |                       |
  |                              |<-----------------------------|                       |                       |
  |  CLI prints Accepted line    |                              |                       |                       |
  |<-----------------------------|                              |                       |                       |
  |                              |                              | dispatch StartAllocation                      |
  |                              |                              |---------------------->|                       |
  |                              |                              |                       | exec workload         |
  |                              |                              |                       | write AllocStatusRow{Running} (kernel-accepted)
  |                              |                              |                       |---------------------->|
  |                              |                              |                       | start ProbeRunner     |
  |                              |                              |                       | tick: probe startup   |
  |                              |                              |                       | (TCP connect / HTTP GET / exec)
  |                              |                              |                       | write ProbeResultRow  |
  |                              |                              |                       |---------------------->|
  |                              |                              | ServiceLifecycleReconciler ticks              |
  |                              |                              | reads ProbeResultRow, AllocStatusRow          |
  |                              |                              | startup PASS for K consecutive ticks?         |
  |                              |                              | -> emit Action::SetTerminalCondition{Stable}  |
  |                              |                              | action shim writes AllocStatusRow.terminal=Stable
  |                              |                              | broadcasts LifecycleEvent.terminal=Stable     |
  |                              |   ServiceSubmitEvent::Stable |                       |                       |
  |                              |   { settled_in: 1.2s,        |                       |                       |
  |                              |     witness: StartupProbe { id: 0, uri: "..." } }    |                       |
  |                              |<-----------------------------|                       |                       |
  |  "Service 'payments' is stable (took 1.2s, startup probe http://...:8080/healthz)"  |                       |
  |<-----------------------------|                              |                       |                       |
  |   exit 0                     |                              |                       |                       |
```

## Failure path ASCII (RCA-A closing)

```
[Ana]                       [CLI]                     [server]                       [worker]
  |  submit coinflip.toml as Service                              | parse as ServiceSpec (no listeners declared = parse error
  |                              |   400 ParseError::NoListeners  |   per ADR-0047; out of scope for THIS feature)
```

For the in-scope Service early-exit case:

```
[Ana]                       [CLI]                     [server]                       [worker]
  |  submit slow-bind.toml      |                              |                       |
  |---------------------------->|                              |                       |
  |                             |  POST /v1/submit             |                       |
  |                             |----------------------------->|                       |
  |                             |  200 + Accepted              | StartAllocation        |
  |                             |<-----------------------------|------------------------>|
  |                             |                              |                       | exec, exit 1 within 50ms
  |                             |                              |                       | ExitObserver writes AllocStatusRow{Failed, exit_code: 1}
  |                             |                              | ServiceLifecycleReconciler ticks
  |                             |                              | sees Failed BEFORE startup probe ever passed
  |                             |                              | within startup_deadline window
  |                             |                              | -> emit Action::SetTerminalCondition{
  |                             |                              |      Failed { reason: EarlyExit { exit_code: 1 } } }
  |                             | ServiceSubmitEvent::Failed   |                       |
  |                             |   { reason: EarlyExit { 1 }, |                       |
  |                             |     stderr_tail: "..." }     |                       |
  |                             |<-----------------------------|                       |
  | "Service 'slow-bind' failed: workload exited with code 1 within startup deadline (1.2s after submit)
  |  stderr: ERROR: failed to bind 0.0.0.0:8080: address already in use"
  |<----------------------------|                              |                       |
  |   exit 1                    |                              |                       |
```

## TUI mockups per step

### Step 1 — Accepted line (unchanged from existing streaming submit)

```
+-- Step 1: submit ---------------------------------------------------------+
| $ overdrive job submit payments.toml                                       |
|                                                                           |
| Accepted: service 'payments' (intent_key=service/payments, commit=42)      |
|                                                                           |
+---------------------------------------------------------------------------+
```

### Step 2 — Progress (NEW — Slice 06 surface; preview here for journey coherence)

```
+-- Step 2: probing (NEW; rendered when --progress or interactive TTY) -----+
| Accepted: service 'payments' (intent_key=service/payments, commit=42)      |
| Probing startup [tcp 127.0.0.1:8080] attempt 1/30, last: connection refused|
| Probing startup [tcp 127.0.0.1:8080] attempt 2/30, last: connection refused|
| Probing startup [tcp 127.0.0.1:8080] attempt 3/30, last: ok                |
| Probing startup [tcp 127.0.0.1:8080] attempt 4/30, last: ok                |
|                                                                           |
+---------------------------------------------------------------------------+
```

`${alloc_id}` is the canonical id; `${probe_idx}` is the TOML array position (or `0` for the inferred default).

### Step 3a — Stable terminal (happy path)

```
+-- Step 3a: Stable -------------------------------------------------------+
| Accepted: service 'payments' (intent_key=service/payments, commit=42)     |
| Service 'payments' is stable                                              |
|   settled_in: 1.2s                                                        |
|   witness:    startup probe #0 (tcp 127.0.0.1:8080)                       |
|                                                                          |
| $ echo $?                                                                |
| 0                                                                        |
+--------------------------------------------------------------------------+
```

`${settled_in}` is a `Duration` computed by the Service reconciler at the deciding tick (the difference between `AllocStatusRow.started_at` and `now`). `${witness}` names which probe (by `probe_idx` and `role`) crossed the threshold.

### Step 3b — Failed terminal (startup probe timed out)

```
+-- Step 3b: Failed (StartupProbeFailed) ----------------------------------+
| Accepted: service 'payments' (intent_key=service/payments, commit=42)     |
| Service 'payments' failed: startup probe timed out                        |
|   probe:      startup #0 (http GET http://127.0.0.1:8080/healthz)         |
|   attempts:   30/30                                                       |
|   last_fail:  HTTP 503 after 1.4s                                         |
|   elapsed:    60.0s (startup_deadline=60s)                                |
|                                                                          |
| Run 'overdrive alloc status --job payments' for full probe history.       |
|                                                                          |
| $ echo $?                                                                |
| 1                                                                        |
+--------------------------------------------------------------------------+
```

### Step 3c — Failed terminal (EarlyExit; RCA-A close for coinflip-shape workloads)

```
+-- Step 3c: Failed (EarlyExit) -------------------------------------------+
| Accepted: service 'coinflip-as-service' (intent_key=service/cf, commit=7) |
| Service 'coinflip-as-service' failed: workload exited within startup      |
| deadline                                                                  |
|   exit_code:        1                                                     |
|   elapsed:          0.05s (startup_deadline=60s)                          |
|   stderr_tail:      "ERROR"                                               |
|                                                                          |
| The workload exited before any startup probe could pass. Inspect the      |
| spec's command, environment, or listener configuration.                   |
|                                                                          |
| $ echo $?                                                                |
| 1                                                                        |
+--------------------------------------------------------------------------+
```

### Step 4 — `alloc status` Probes section (Slice 06)

```
+-- Step 4: alloc status (Probes section, rendered only for Service kind) -+
| $ overdrive alloc status --job payments                                   |
|                                                                          |
| Service: payments                                                         |
|   spec_digest:  sha256:abcd…                                              |
|   replicas:     1/1 stable                                                |
|   stable_since: 2026-05-24T18:42:13Z                                      |
|                                                                          |
| Allocations:                                                              |
|   alloc-payments-0   state=Running   terminal=Stable                      |
|     Probes:                                                               |
|       startup   #0  tcp 127.0.0.1:8080         last=ok    at 18:42:11Z   |
|       readiness #0  http GET /healthz          last=ok    at 18:42:43Z   |
|       liveness  #0  http GET /healthz          last=ok    at 18:42:43Z   |
|                                                                          |
+--------------------------------------------------------------------------+
```

Probes section is present iff `kind == Service` AND `alloc has at least one probe declared or inferred`.

### Step 5 — Default-probe TOML inference (operator declares NO probes)

```
+-- Step 5: minimal Service spec --------------------------------------------+
| # payments-minimal.toml                                                    |
| [service]                                                                  |
| id = "payments-minimal"                                                    |
| replicas = 1                                                               |
|                                                                            |
| [[listener]]                                                               |
| port = 8080                                                                |
|                                                                            |
| [exec]                                                                     |
| command = ["python", "-m", "http.server", "8080"]                          |
|                                                                            |
| # NO [[health_check.*]] sections — platform infers a default TCP-connect   |
| # startup probe against the first listener port (8080) with default        |
| # timeout/interval. Operator sees `(probe inferred: tcp 0.0.0.0:8080)` in  |
| # the streaming progress and in `alloc status`.                            |
+----------------------------------------------------------------------------+
```

## Shared artifacts (preview — full registry in `shared-artifacts-registry.md`)

| Artifact | Source of truth | Consumers | Risk |
|---|---|---|---|
| `${probe_idx}` | TOML array position OR `0` for inferred default | ProbeResultRow PK; CLI render; streaming progress line | HIGH — must match across all surfaces or operator loses trail |
| `${settled_in}` | Service reconciler's deciding tick (`now - AllocStatusRow.started_at`) | `ServiceSubmitEvent::Stable` wire variant; CLI render | MEDIUM — must be a real `Duration`, not a sentinel literal |
| `${witness}` | Service reconciler's deciding tick (probe_idx + role that crossed threshold) | `ServiceSubmitEvent::Stable.witness` wire variant; CLI render | MEDIUM — operator needs to know which probe agreed |
| `${last_observed_at}` | ProbeResultRow (LWW per `(alloc_id, probe_idx)`) | CLI `alloc status` Probes section | LOW |
| `${last_fail_reason}` | ProbeResultRow (string captured by ProbeRunner on each failure) | CLI Probes section (last_fail column); `ServiceSubmitEvent::Failed.reason` payload | HIGH — must be stable enough for operator to act |
| `startup_deadline` | TOML `[[health_check.startup]].timeout_seconds × .max_attempts` OR platform default (60s) | Service reconciler (decides Failed window); CLI render (elapsed/deadline pair) | MEDIUM — operator-facing duration |

## Integration checkpoints

- After Step 1 (Accepted): `intent_key` + `commit_index` byte-equal between CLI line and server's IntentStore write. Reuses existing checkpoint from `submit-a-job` journey.
- After Step 2 (Probing progress): each progress line's `probe_idx` MUST exist in the spec's `[[health_check.*]]` arrays OR equal `0` for the inferred-default case.
- After Step 3 (Stable/Failed): `settled_in` is a real `Duration::from_millis(N)` where N > 0; `witness.probe_idx` resolves to a declared/inferred probe.
- After Step 4 (`alloc status` render): Probes section iff `kind == Service AND probes_present`. For `Job` and `Schedule` kinds, the section MUST be absent (kind-rejection rules out probes on those kinds at parse time).

## Failure modes (per step, for DISTILL test scenario generation)

### Step 1 (Submit)
- TOML contains `[[health_check.*]]` under `[job]` → `ParseError::ProbesNotAllowedOnKind { kind: "job", guidance: "Job has no readiness question; on completion is enough." }`
- TOML contains `[[health_check.*]]` under `[schedule]` → `ParseError::ProbesNotAllowedOnKind { kind: "schedule", guidance: "Schedule composes per-fire; declare probes on the underlying [job] (rejected too) — Schedule cannot carry probes." }`
- Service spec has zero `[[listener]]` AND no explicit probes → already rejected by ADR-0047 `ParseError::NoListeners`; default-probe inference cannot fire.
- HTTP probe path missing → `ParseError::HttpProbeMissingPath { probe_idx }`.
- Exec probe command empty → `ParseError::ExecProbeMissingCommand { probe_idx }`.

### Step 2 (Probe wait)
- ProbeRunner cannot dispatch (worker offline) → ProbeResultRow stays absent; Service reconciler treats absent-row as `Pending` for `startup_deadline` window then emits `Failed { reason: ProbeRunnerUnreachable }`.
- HTTP probe target binds but returns 5xx persistently → counts as fail per attempt; eventual `StartupProbeFailed` with `last_fail` rendered.
- Exec probe binary not found in workload's cgroup → counted as fail with `last_fail: "exec: command not found"`.

### Step 3 (Stable/Failed)
- Workload exits 0 within startup_deadline (Service that race-completes) → Service-kind treats early-exit as failure regardless of exit code, because a Service is expected to be long-lived; `Failed { reason: EarlyExit { exit_code: 0 } }`.
- Streaming cap (60s default) fires before startup probe completes → existing `Timeout` reason path; this feature does NOT extend `streaming_cap`. Operator can still query `alloc status` to see in-progress probe state.

### Step 4 (alloc status)
- Probes section requested for a Job or Schedule alloc → MUST NOT render the section (renderer-side guard).

### Step 5 (default-probe inference)
- Operator declares an empty `[[health_check.startup]] = []` (intentional override) → MUST be respected as "no startup probe; alloc reaches Stable on first Running row" (preserves current Phase-1 behaviour as an explicit opt-out for operators who genuinely want the old shape).
- Operator declares listeners on multiple ports without explicit startup probe → inferred default picks the FIRST `[[listener]]` only; documented behaviour. (Operators wanting per-port probes declare them explicitly.)

## Changelog

- 2026-05-24 — Initial journey visual for service-health-check-probes DISCUSS wave. Walking skeleton = Slice 01 default TCP-connect startup probe.
