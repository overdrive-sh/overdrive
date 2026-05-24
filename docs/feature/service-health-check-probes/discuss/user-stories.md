<!-- markdownlint-disable MD024 -->

# User Stories — service-health-check-probes

**Feature:** `service-health-check-probes`
**Wave:** DISCUSS
**Source brief:** GH #170; predecessor RCA `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`
**Job:** J-OPS-004 (Service-honesty; extends J-OPS-003)

## System Constraints (cross-cutting)

These constraints apply to every story in this feature. They are NOT acceptance criteria of any individual story — they bind the design space for ALL of them and DESIGN/DELIVER must honour them.

- **C1. Probe runner lives in the WORKER process**, not the control plane. Per GH #170 and `.claude/rules/development.md` — control plane converges intent; observation comes from the machine running the workload. Probe results flow as ObservationStore rows.
- **C2. Probe results are observation, never intent.** ProbeResultRow lives in `ObservationStore`. Reconcilers read; never write probe outcomes directly.
- **C3. LWW per `(alloc_id, probe_idx)`.** ProbeResultRow row identity is the pair; latest-writer-wins. No append-mode per-tick history (per `.claude/rules/development.md` § "Persist inputs, not derived state" — operational history is computed at read time from the latest plus the policy, never persisted).
- **C4. Default policy is operator-overridable but not removable.** When TOML omits `[[health_check.startup]]` AND has at least one `[[listener]]`, platform infers a single TCP-connect startup probe (probe_idx = 0) against the first listener port. An empty `[[health_check.startup]] = []` array is the explicit opt-out (preserves current Phase-1 first-Running semantics).
- **C5. Service-kind only.** TOML containing `[[health_check.*]]` under `[job]` or `[schedule]` MUST be rejected at parse time with `ParseError::ProbesNotAllowedOnKind { kind, guidance }` naming the right primitive.
- **C6. HTTP probes are PLAIN HTTP for now.** HTTPS / mTLS / gRPC are explicit non-goals (deferred to Phase 3+ per GH #170; whitepaper §11 sockops+kTLS).
- **C7. Exec probes run inside the workload cgroup.** Exec runs as a member of `/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-<id>.scope` — not a sibling, not the worker's cgroup.
- **C8. Wire byte-equality (ADR-0037 §3).** `AllocStatusRow.terminal` and `LifecycleEvent.terminal` are populated by the same action-shim call site with the same value. No drift.
- **C9. No `"live"` sentinel in operator-facing output.** Per RCA-A solution D — `settled_in` is a real `Duration`. Render must format it as e.g. `1.2s`, never the literal string `"live"`.
- **C10. Streaming cap (60s default) is unchanged.** Slow-warming Services may exceed cap and emit existing `ConvergedFailed { Timeout }`; operator inspects `alloc status` for in-progress probe state.
- **C11. Liveness probes check application-internal health only.** Liveness verifies deadlock/hang conditions within the workload process, never downstream dependency availability (database, cache, message broker). Dependency health failures must be caught by readiness probes, not liveness. Per Kubernetes best practice; prevents cascading restarts when shared dependencies degrade. Reference: research § 6.1 Pitfall 1+2, § 6.2 best practice 1.
- **C12. Initial delay on probes is deferred.** Phase 1 executes probes immediately after alloc spawn. A configurable initial-delay field (Kubernetes `initialDelaySeconds`, Nomad `grace`) is deferred to Phase 2.
- **C13. Declare readiness to prevent premature traffic during application initialization.** A Service without explicit `[[health_check.readiness]]` has all backends marked `healthy = true` once Stable is reached (startup probe passed). Applications requiring initialization time beyond the startup probe should declare readiness. Reference: research § 6.1 Pitfall 4.

---

## US-01: Operator submits a probe-less Service and trusts the Stable / Failed signal (WALKING SKELETON)

### Elevator Pitch

- **Before:** Ana submits `payments-minimal.toml` (a Service with one listener, no probes declared) and receives `is running with 1/1 replicas (took live)` even when her workload exits with code 1 within 50ms — the CLI lies.
- **After:** Ana runs `overdrive job submit payments-minimal.toml` and sees `Service 'payments-minimal' is stable\n  settled_in: 1.2s\n  witness: startup probe #0 (tcp 0.0.0.0:8080)` on success OR `Service 'payments-minimal' failed: workload exited with code 1 within startup deadline (0.05s after submit)\n  stderr_tail: "ERROR"` on early-exit — honest in both cases.
- **Decision enabled:** Ana decides whether her Service is fit to receive traffic, or whether she needs to debug the entrypoint, based on the wire signal alone — without resorting to `sleep 5 && alloc status`.

### Problem

Ana Lopez is an Overdrive platform engineer who, on 2026-05-09, ran `overdrive job submit examples/coinflip.toml` against her single-node dev host and watched the CLI print `Job 'coinflip' is running with 1/1 replicas (took live)` exit code 0, despite the `serve` log showing the workload had alternately printed `SUCCESS` and `ERROR` (exits 0 and 1). She found it impossible to trust the streaming submit's "Running" claim to indicate operator-meaningful liveness. Her current workaround is `submit && sleep N && alloc status`, which is awkward to script and prone to false confidence when N is too small.

### Who

- **Ana Lopez** — Overdrive platform engineer on a single-node dev host. Reads JSON-shaped CLI output. Tolerates 1-2s of submit latency in exchange for honest signal.
- **Context:** Ana is iterating on a Service spec for `payments` — she submits, observes failure, edits spec, re-submits, 5-10 times per hour.
- **Motivation:** She wants the CLI to be the SSOT of truth. If submit prints Stable, the Service is serving; if submit prints Failed, she knows what to fix.

### Solution

The platform infers a default TCP-connect startup probe (probe_idx = 0) against the first declared listener port when the Service TOML omits `[[health_check.startup]]`. A ProbeRunner in the worker ticks the probe at a default interval; results land in the ObservationStore as `ProbeResultRow`. A new `ServiceLifecycleReconciler` reads the probe result and the alloc state, and emits one of two new TerminalCondition variants: `Stable { settled_in, witness }` (operator-meaningful liveness reached) or `Failed { reason }` where `reason` is `StartupProbeFailed` or `EarlyExit { exit_code }`. The streaming subscriber maps these to new `ServiceSubmitEvent::Stable` / `ServiceSubmitEvent::Failed` wire variants. The CLI renders honest lines containing a real `Duration` and a named witness probe.

### Domain Examples

#### 1: Happy path — payments-minimal with quick-binding listener

Ana writes `payments-minimal.toml`:

```toml
[service]
id = "payments-minimal"
replicas = 1

[[listener]]
port = 8080

[exec]
command = ["python", "-m", "http.server", "8080"]
```

She runs `overdrive job submit payments-minimal.toml`. The `python -m http.server` process binds 0.0.0.0:8080 within ~600ms. The inferred default TCP probe connects successfully on its second tick at T0+800ms. The Service reconciler decides Stable at T0+900ms.

CLI output:
```
Accepted: service 'payments-minimal' (intent_key=service/payments-minimal, commit=42)
Service 'payments-minimal' is stable
  settled_in: 0.9s
  witness:    startup probe #0 (tcp 0.0.0.0:8080)
```
Exit code 0.

#### 2: Edge case — slow-warming Service whose listener binds at T0+45s

Ana writes `jvm-app.toml` with a Java service whose JVM warmup takes 30-40s before the listener binds. Default TCP probe attempts connect repeatedly; first 20 attempts get `connection refused`. At T0+45s the listener binds; next probe tick (T0+46s) succeeds. Stable emitted at T0+46s.

```
Accepted: service 'jvm-app' (intent_key=service/jvm-app, commit=43)
Service 'jvm-app' is stable
  settled_in: 46.0s
  witness:    startup probe #0 (tcp 0.0.0.0:8080)
```

#### 3: Error — workload exits 1 within 50ms (the RCA-A coinflip case, reshaped as Service)

Ana writes `coinflip-as-service.toml`:

```toml
[service]
id = "coinflip-as-service"
replicas = 1

[[listener]]
port = 8080

[exec]
command = ["/bin/bash", "-c", "echo ERROR >&2 && exit 1"]
```

The exec runs, exits 1 at T0+30ms before the default TCP probe gets to fire. ExitObserver writes terminal `AllocStatusRow { state: Failed, exit_code: 1 }`. Service reconciler sees the Failed row WITHIN startup_deadline window AND with NO Pass ProbeResultRow → emits Failed { reason: EarlyExit { exit_code: 1 } }.

```
Accepted: service 'coinflip-as-service' (intent_key=service/cf, commit=44)
Service 'coinflip-as-service' failed: workload exited within startup deadline
  exit_code:    1
  elapsed:      0.05s (startup_deadline=60s)
  stderr_tail:  "ERROR"

The workload exited before any startup probe could pass. Inspect the
spec's command, environment, or listener configuration.
```
Exit code 1.

### UAT Scenarios (BDD)

#### Scenario: Probe-less Service reaches Stable when listener binds within startup deadline
```gherkin
Given Ana has authored payments-minimal.toml with a [service] block, one [[listener]] on port 8080, and no [[health_check.*]] sections
And the [exec] command binds 0.0.0.0:8080 within 1 second of start
When Ana runs `overdrive job submit payments-minimal.toml`
Then the CLI prints "Accepted: service 'payments-minimal' ..."
And within 2 seconds the CLI prints "Service 'payments-minimal' is stable"
And the next line contains "settled_in:" followed by a Duration of the form "<N>s" with N > 0
And the next line contains "witness:    startup probe #0 (tcp 0.0.0.0:8080)"
And the exit code is 0
```

#### Scenario: Probe-less Service emits Failed (EarlyExit) when workload exits before probe passes
```gherkin
Given Ana has authored coinflip-as-service.toml with a [service] block, one [[listener]] on port 8080, no probes, and an [exec] command that exits with code 1 within 50ms
When Ana runs `overdrive job submit coinflip-as-service.toml`
Then within 5 seconds the CLI prints "Service 'coinflip-as-service' failed: workload exited within startup deadline"
And the output contains "exit_code:    1"
And the output contains "elapsed:" followed by a Duration shorter than the startup_deadline
And the exit code is 1
And the output does NOT contain the substring "is running"
And the output does NOT contain the substring "(took live)"
```

#### Scenario: Probe-less Service emits Failed (StartupProbeFailed) when listener never binds within deadline
```gherkin
Given Ana has authored slow-bind.toml with a [service] block, one [[listener]] on port 8080, and an [exec] command that takes 120 seconds to bind the listener (longer than startup_deadline default 60s)
When Ana runs `overdrive job submit slow-bind.toml`
Then within 65 seconds the CLI prints "Service 'slow-bind' failed: startup probe timed out"
And the output names the probe ("startup #0 (tcp 0.0.0.0:8080)")
And the output shows the last failure reason ("connection refused")
And the exit code is 1
```

#### Scenario: AllocStatusRow.terminal and LifecycleEvent.terminal carry byte-identical Stable value
```gherkin
Given a Service alloc has reached Stable per the reconciler's deciding tick
When the test harness reads the AllocStatusRow.terminal field AND captures the LifecycleEvent.terminal field from the broadcast bus
Then the two TerminalCondition values are byte-equal under rkyv-archive serialisation
```

#### Scenario: The inferred default probe is visible in `alloc status` (cross-link to US-06)
```gherkin
Given a Stable Service 'payments-minimal' submitted without explicit probes
When Ana runs `overdrive alloc status --job payments-minimal`
Then the output's Probes section contains exactly one row with "startup #0" and mechanic "tcp 0.0.0.0:8080" and a marker indicating "(inferred)"
```

### Acceptance Criteria

- [ ] Service spec with zero `[[health_check.*]]` sections AND ≥1 `[[listener]]` infers exactly one TCP-connect startup probe at probe_idx = 0
- [ ] Stable wire event carries `settled_in: Duration` (real measurement, never sentinel) and `witness` naming probe_idx + role
- [ ] Failed wire event carries `reason: EarlyExit { exit_code } | StartupProbeFailed { probe_idx, last_fail, attempts }`
- [ ] CLI exit code 0 for Stable, 1 for Failed (any reason)
- [ ] CLI render NEVER emits the literal "(took live)" string for Service-kind allocs
- [ ] AllocStatusRow.terminal and LifecycleEvent.terminal carry byte-equal TerminalCondition values for the same deciding tick
- [ ] Inferred-default probe renders in `alloc status` Probes section with `(inferred)` marker (Slice 06 surface)

### Outcome KPIs

- **Who**: Operator submitting a Service whose workload either crashes within startup deadline or whose startup probe never passes
- **Does what**: Sees `ServiceSubmitEvent::Failed { reason: StartupProbeFailed | EarlyExit }` (NOT `Stable`, NOT bare `Running`) within `startup_deadline + 1 tick`
- **By how much**: ≥99 of 100 such submissions (≥99%)
- **Measured by**: Integration test `crates/overdrive-cli/tests/integration/service_honest_stable.rs` reshapes coinflip.toml as Service with never-passing startup probe; parses CLI output
- **Baseline**: 0% (Phase 1 reports Stable-equivalent for the entire kernel-accepted window — RCA-A failure mode)

### Technical Notes

- New trait: `ProbeRunner` in `overdrive-worker` (TCP-only for this slice — HTTP / Exec come in US-02 / US-03).
- New observation row: `ProbeResultRow` in `overdrive-core` (LWW per `(alloc_id, probe_idx)`).
- New TerminalCondition variants: `Stable { settled_in, witness }` and `Failed { reason: EarlyExit | StartupProbeFailed }` — additive on ADR-0037's `#[non_exhaustive]` enum. SemVer additive minor.
- New wire variants: `ServiceSubmitEvent::Stable` / `ServiceSubmitEvent::Failed` per ADR-0032 Amendment 2026-05-10 (extends the per-kind enum).
- Reconciler split: extract `ServiceLifecycleReconciler` from current `JobLifecycleReconciler` per ADR-0047.
- Default probe descriptor: `Tcp { host: "0.0.0.0", port: listeners[0].port }, timeout: 5s, interval: 2s, max_attempts: 30` → `startup_deadline = 60s`.
- Empty `[[health_check.startup]] = []` is the explicit opt-out path (preserves current behaviour); MUST be parser-tested.
- Startup budget = `max_attempts × interval_seconds` (e.g., 30 × 2s = 60s default). Operators requiring longer startup increase `max_attempts` or `interval_seconds` per spec. This matches Kubernetes' `failureThreshold × periodSeconds` convention; reference: research § 2.1, § 7.2 D3.

### Dependencies

- ADR-0047 (workload kind discriminator) — landed; provides the Service / Job split.
- ADR-0037 (`TerminalCondition` enum) — landed; this story adds variants.
- ADR-0032 (NDJSON streaming + per-kind ServiceSubmitEvent) — landed.
- ADR-0033 (`alloc_status` enrichment) — landed; Probes section is US-06.

---

## US-02: Operator declares an HTTP startup probe and gets Stable based on the probe's response

### Elevator Pitch

- **Before:** Ana cannot tell the platform "stable means HTTP 200 on /healthz, not just TCP-connect-succeeds". For a Service that binds the port immediately but takes 10s to initialise the application layer, current Phase 1 (and US-01's TCP default) declares Stable too early.
- **After:** Ana runs `overdrive job submit payments-with-http-probe.toml` whose TOML carries `[[health_check.startup]] type = "http", path = "/healthz", port = 8080`. The CLI prints `Service 'payments' is stable\n  settled_in: 10.4s\n  witness: startup probe #0 (http GET http://0.0.0.0:8080/healthz)` after the workload's `/healthz` endpoint returns 2xx.
- **Decision enabled:** Ana decides what "ready to receive traffic" means for her service in application terms, not in TCP-listener terms.

### Problem

Ana's `payments` Service binds its HTTP port within 200ms but takes 8-10s to load its routing configuration from an upstream config service. During the 8-10s gap, the listener accepts connections but returns HTTP 503. The default TCP-connect probe from US-01 declares Stable at T+200ms — wrongly. Ana wants to declare "Stable means /healthz returns 2xx", which is the standard k8s shape every operator already knows.

### Who

- **Ana Lopez** — same as US-01. Has worked with k8s before and expects `httpGet` probe shape parity.
- **Context:** Ana's Service has a non-trivial startup whose readiness is application-layer, not TCP-layer.
- **Motivation:** Match k8s muscle memory; declare "ready" in HTTP terms.

### Solution

Extend the TOML parser to accept `[[health_check.startup]] type = "http", path = "<path>", port = <u16>, timeout_seconds = <u32>, interval_seconds = <u32>, max_attempts = <u32>` (sensible defaults for everything except `path` and `port`). Extend `ProbeRunner` to dispatch HTTP GET. The 2xx response within timeout counts as Pass; non-2xx (including 5xx) AND connection error AND timeout all count as Fail. The HTTP body is ignored. Last fail reason is one of: `HTTP <code>`, `connection refused`, `timeout after <duration>`.

### Domain Examples

#### 1: Happy path — payments with /healthz HTTP probe
TOML:
```toml
[service]
id = "payments"
replicas = 1

[[listener]]
port = 8080

[[health_check.startup]]
type = "http"
path = "/healthz"
port = 8080
timeout_seconds = 5
interval_seconds = 2
max_attempts = 30

[exec]
command = ["/usr/local/bin/payments-server"]
```
The workload binds 8080 at T0+0.2s; `/healthz` returns 503 ("config loading...") until T0+8.4s, then returns 200 ("ready"). Probe ticks at T0+2s, T0+4s, T0+6s, T0+8s (Fail), then T0+10s (Pass). Stable at T0+10.1s.

```
Accepted: service 'payments' ...
Service 'payments' is stable
  settled_in: 10.1s
  witness:    startup probe #0 (http GET http://0.0.0.0:8080/healthz)
```

#### 2: Edge case — HTTP probe with non-default path AND probe runs against a different port than the listener
TOML carries `[[listener]] port = 8080` (the public listener) and `[[health_check.startup]] type = "http", path = "/internal/ready", port = 9090`. The probe targets the admin port 9090, distinct from the public 8080. Stable based on 9090's `/internal/ready` returning 2xx.

#### 3: Error — probe path returns persistent 503
The workload binds successfully but `/healthz` returns 503 for every attempt within startup_deadline. CLI prints:
```
Service 'payments' failed: startup probe timed out
  probe:      startup #0 (http GET http://0.0.0.0:8080/healthz)
  attempts:   30/30
  last_fail:  HTTP 503
  elapsed:    60.0s (startup_deadline=60s)
```

### UAT Scenarios (BDD)

#### Scenario: HTTP startup probe with 2xx response transitions Service to Stable
```gherkin
Given Ana has authored payments.toml with a [[health_check.startup]] of type "http", path "/healthz", port 8080
And the workload returns HTTP 503 for the first 8 seconds, then HTTP 200
When Ana runs `overdrive job submit payments.toml`
Then within 12 seconds the CLI prints "Service 'payments' is stable"
And the witness line contains "http GET http://0.0.0.0:8080/healthz"
And settled_in is at least 8 seconds
```

#### Scenario: HTTP probe with persistent 503 emits Failed with HTTP-503 last_fail
```gherkin
Given an HTTP startup probe on /healthz port 8080
And the workload returns HTTP 503 for every probe attempt within startup_deadline
When startup_deadline elapses
Then the CLI prints "Service 'payments' failed: startup probe timed out"
And the output contains "last_fail:  HTTP 503"
And the exit code is 1
```

#### Scenario: HTTP probe with connection refused records named reason
```gherkin
Given an HTTP startup probe on port 8080
And the workload never binds the listener
When the probe ticks
Then ProbeResultRow has status Fail with last_fail_reason "connection refused"
```

#### Scenario: HTTP probe missing path field is rejected at parse time
```gherkin
Given a TOML containing `[[health_check.startup]]\ntype = "http"\nport = 8080` (no path)
When the parser processes it
Then a ParseError::HttpProbeMissingPath { probe_idx: 0 } is returned
And the CLI prints an error naming the missing field
```

### Acceptance Criteria

- [ ] HTTP probe TOML shape parses with required fields `type = "http"`, `path`, `port`; optional `timeout_seconds` (default 5), `interval_seconds` (default 2), `max_attempts` (default 30)
- [ ] HTTP 2xx response within timeout = Pass; HTTP non-2xx, connection error, or timeout = Fail with named `last_fail_reason`
- [ ] Probe targets `http://<bind_host>:<configured_port><path>` — port may differ from any `[[listener]].port`
- [ ] Missing `path` → `ParseError::HttpProbeMissingPath { probe_idx }` at parse time
- [ ] Stable wire event names the witness as "http GET http://...<path>"
- [ ] HTTP 3xx (redirect) responses are treated as Fail; the probe does NOT follow redirects. Rationale: prevents HTTPS-redirect loops from masking health failures and matches Fly.io behavior. Reference: research § 6.1 Pitfall 5.
- [ ] HTTP method is GET only in Phase 1. POST and custom methods are deferred. Probe request has no body.

### Outcome KPIs

- **Who**: Operator declaring an HTTP startup probe
- **Does what**: Sees Stable only after probe responds 2xx within timeout
- **By how much**: 100% of HTTP probe Pass observations precede Stable emission (zero false-Stable on pre-Pass ticks)
- **Measured by**: Acceptance test with controllable HTTP server fixture
- **Baseline**: N/A (HTTP probe doesn't exist)

### Technical Notes

- Plain HTTP only (C6). No HTTPS, no mTLS, no client cert, no retries-with-backoff inside a single attempt.
- HTTP client: lightweight blocking shape inside ProbeRunner (e.g. `hyper` over an injected `Transport`); ProbeRunner is per-alloc per-tick.
- Body is ignored — only status code matters.
- HTTP method: GET only. POST and custom methods deferred to Phase 2.
- Redirect handling: 3xx responses count as failure (same treatment as 4xx/5xx). The probe does NOT follow redirects.

### Dependencies

- US-01 (walking skeleton — ProbeRunner trait, ProbeResultRow, Service reconciler, new TerminalCondition variants, new wire variants).

---

## US-03: Operator declares an Exec startup probe that runs inside the workload cgroup

### Elevator Pitch

- **Before:** Ana wants to declare "Stable means `/usr/local/bin/healthcheck.sh --strict` exits 0 from inside the workload's network/mount namespace" but has no platform-supported mechanism — must shell-script it externally.
- **After:** Ana runs `overdrive job submit payments-with-exec-probe.toml` whose TOML carries `[[health_check.startup]] type = "exec", command = ["/usr/local/bin/healthcheck.sh", "--strict"]`. The CLI prints Stable once the exec exits 0.
- **Decision enabled:** Ana writes domain-specific health logic in any language and the platform runs it AS the workload.

### Problem

Ana's `payments` Service has a health check that requires: (a) reading the workload's `/etc/payments/config.toml`, (b) probing an upstream database from the workload's network namespace, (c) verifying a local cache file exists. None of these are reachable from the worker process — they require the workload's mount + network namespace. Today she has no platform-supported way to declare such a probe; she ad-hocs it.

### Who

- **Ana Lopez** — same as US-01.
- **Context:** Service needs domain-specific health logic involving the workload's namespaces and filesystem.
- **Motivation:** Avoid having to wedge health-check logic into the application's entrypoint as a sidecar shell loop.

### Solution

Extend TOML parser to accept `[[health_check.startup]] type = "exec", command = [...], timeout_seconds = <u32>, ...`. Extend `ProbeRunner` to spawn the command as a member of the workload's cgroup scope (per C7). Exit 0 → Pass; non-zero exit OR timeout → Fail with last_fail_reason `exit <N>` or `timeout after <duration>` or `exec: command not found`.

### Domain Examples

#### 1: Happy path
TOML has `command = ["/usr/local/bin/healthcheck.sh", "--strict"]`. Script exits 0. Stable emitted.

#### 2: Edge — command missing in image
TOML has `command = ["/usr/local/bin/nonexistent"]`. ProbeRunner fails to spawn. ProbeResultRow has `last_fail_reason: "exec: command not found"`. Eventually Failed { StartupProbeFailed { last_fail: "exec: command not found", ... } }.

#### 3: Error — script times out
TOML has `command = ["/bin/sleep", "999"]`, `timeout_seconds = 5`. Each tick times out after 5s, Fail with `last_fail_reason: "timeout after 5s"`. Eventual Failed.

### UAT Scenarios (BDD)

#### Scenario: Exec startup probe exit 0 produces Pass
```gherkin
Given a Service alloc with an exec startup probe ["/bin/true"]
When the ProbeRunner ticks
Then ProbeResultRow has status Pass
```

#### Scenario: Exec probe runs as a member of the workload cgroup
```gherkin
Given a Service alloc 'alloc-payments-0' with an exec probe that prints its own cgroup membership to stderr
And the worker cgroup is /sys/fs/cgroup/overdrive.slice/control.slice/worker.scope
When the ProbeRunner runs the exec probe
Then the probe process's /proc/self/cgroup membership names /sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-payments-0.scope
And does NOT name the worker scope
```

#### Scenario: Exec probe with missing command captures named failure
```gherkin
Given an exec probe with command ["/usr/local/bin/nonexistent"]
When the ProbeRunner attempts to spawn
Then the ProbeResultRow has last_fail_reason "exec: command not found"
```

#### Scenario: Exec probe respects timeout
```gherkin
Given an exec probe ["/bin/sleep", "10"] with timeout_seconds = 2
When the ProbeRunner ticks at T0
Then the probe is killed at T0 + 2s
And the ProbeResultRow has last_fail_reason "timeout after 2s"
```

### Acceptance Criteria

- [ ] Exec probe TOML shape parses with required `type = "exec"`, `command: [String]`; optional `timeout_seconds`
- [ ] Probe process is member of `alloc-<id>.scope` cgroup (asserted via /proc/<pid>/cgroup on Linux integration test)
- [ ] Empty command array → `ParseError::ExecProbeMissingCommand { probe_idx }`
- [ ] Exit 0 = Pass; non-zero exit, spawn-failure, timeout = Fail with named `last_fail_reason`

### Outcome KPIs

- **Who**: Operator declaring an exec startup probe
- **Does what**: Sees probe execute inside workload's cgroup with proper isolation
- **By how much**: 100% of exec probes execute in the workload's cgroup (asserted by /proc/<pid>/cgroup readout)
- **Measured by**: Linux integration test under `integration-tests` feature
- **Baseline**: N/A

### Technical Notes

- Cgroup placement: `clone3` into target cgroup OR `cgroup.procs` write of the spawned PID — DESIGN wave decides. Per `.claude/rules/development.md` § "Production code is not shaped by simulation", `ProbeRunner` trait is the boundary; production binding handles real cgroup, sim binding short-circuits.
- Phase 1 worker is single-machine; exec probe just spawns in the local cgroup. No remote exec.
- Timeout enforced by sending SIGKILL after timeout_seconds; cleanup via existing AllocCleanup-shape Drop guard pattern.

### Dependencies

- US-01.

---

## US-04: Readiness probe failure flips Backend.healthy in the dataplane fingerprint

### Elevator Pitch

- **Before:** A Service backend that has lost its database connection (or otherwise stopped being able to handle requests) continues to receive traffic via the dataplane SERVICE_MAP because there is no per-backend continuous health signal.
- **After:** Ana declares `[[health_check.readiness]]` on her Service. When the readiness probe transitions Pass → Fail, the corresponding `Backend.healthy` flips to `false` within 1 reconciler tick. The dataplane fingerprint changes, and the eBPF SERVICE_MAP / BACKEND_MAP hydrator removes the backend from the load-balanced set.
- **Decision enabled:** Ana relies on the platform to remove unhealthy backends without manual intervention.

### Problem

A `payments` Service running 3 backends. Backend 2 loses its DB connection and starts returning 503. Without readiness-driven health, the dataplane keeps sending traffic to it. Ana wants the platform to notice and stop sending traffic — within seconds, not minutes.

### Who

- **Ana Lopez** — same as prior stories.
- **Context:** Multi-replica Service whose backends may transiently lose dependencies.
- **Motivation:** Eject unhealthy backends from the LB without operator action.

### Solution

Extend TOML parser to accept `[[health_check.readiness]]` array with the same probe-body shape as startup. Readiness probes run continuously (not just during startup). The `ServiceLifecycleReconciler` reads ProbeResultRows for readiness probes and writes `Backend.healthy = readiness_passing` into the `ServiceBackendRow` (the dataplane-fingerprint source). The dataplane hydrator (existing) picks up the row change and reconciles SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP per fingerprint change. Existing `Backend.healthy` field at `crates/overdrive-core/src/dataplane/fingerprint.rs:95` is the consumer.

### Domain Examples

#### 1: Happy path — readiness probe flips healthy=false within 1 tick
3-backend Service; backend 2 readiness HTTP probe starts returning 503 at T0. Next reconciler tick at T0+tick_period writes `Backend{2}.healthy = false`. Fingerprint changes; dataplane reconciles.

#### 2: Edge — readiness flapping doesn't cause restart
Readiness flips Fail → Pass → Fail → Pass on consecutive ticks. Backend.healthy flips with each tick. NO restart is triggered (restarts are liveness-driven; US-05). Operator sees flapping in `alloc status` Probes section.

#### 3: Error — Service with NO readiness declared falls back to startup-derived health
A Service with NO `[[health_check.readiness]]` AND a Stable terminal condition has `Backend.healthy = true` for every backend until Liveness (US-05) or terminal failure removes them. (This preserves backward compatibility for Services that don't care about per-backend readiness.)

### UAT Scenarios (BDD)

#### Scenario: Readiness Pass → Fail flips Backend.healthy within 1 tick
```gherkin
Given a Service 'payments' with 3 backends and a readiness HTTP probe on /healthz
And all 3 backends have status Pass
When backend 2's /healthz transitions to HTTP 503 between two reconciler ticks
Then within tick_period_ms of the next reconciler tick after the Fail row lands, Backend{2}.healthy is false
And the dataplane fingerprint value changes between tick N and tick N+1
```

#### Scenario: Readiness Fail → Pass restores Backend.healthy
```gherkin
Given backend 2 is Backend.healthy = false
When backend 2's readiness probe Passes
Then within 1 reconciler tick Backend{2}.healthy = true again
```

#### Scenario: Service without readiness defaults all backends to healthy when Stable
```gherkin
Given a Service with NO [[health_check.readiness]] declared
And the Service has reached Stable
When the dataplane queries the ServiceBackendRow
Then every backend has healthy = true
```

### Acceptance Criteria

- [ ] Readiness probe TOML shape parses with same body schema as startup (HTTP / TCP / Exec)
- [ ] Reconciler updates `ServiceBackendRow.healthy = (latest_readiness_status == Pass)` on every tick
- [ ] Fingerprint at `crates/overdrive-core/src/dataplane/fingerprint.rs` reflects the change within 1 tick (assert via existing `fingerprint_is_sensitive_to_health_flag` test pattern)
- [ ] Service without readiness probes defaults `Backend.healthy = true` for every backend post-Stable

### Outcome KPIs

K2 — see `outcome-kpis.md`.

### Technical Notes

- Readiness probes are continuous, not bounded by startup_deadline.
- Per-backend readiness, NOT per-alloc — for replicas > 1, each replica's backend has its own readiness row.
- Initial state at alloc spawn: `Backend.healthy = false` until first readiness Pass. Avoids the inverse race (alloc lands, dataplane sees healthy=true, traffic flows, readiness fires Fail).

### Dependencies

- US-01 (runner + ProbeResultRow + Service reconciler).

---

## US-05: Liveness probe consecutive failures past threshold trigger Service restart

### Elevator Pitch

- **Before:** A Service backend that has become wedged (deadlocked, infinite loop, leaked memory) keeps consuming a slot but does no work. Ana must manually `overdrive job stop` and resubmit.
- **After:** Ana declares `[[health_check.liveness]]` on her Service with a `failure_threshold` (e.g. 3 consecutive fails). When threshold is crossed, the platform emits `Action::RestartAllocation` for that alloc. The Service auto-recovers; Ana sees the restart_count tick in `alloc status`.
- **Decision enabled:** Ana decides what "alive" means in domain terms and trusts the platform to restart wedged backends.

### Problem

A `payments` backend deadlocks (database connection pool exhausted; HTTP requests stop being served). Without liveness, the backend stays Running and (per US-04) Backend.healthy = false — but the alloc never recovers. Ana wants the alloc to be killed-and-restarted automatically.

### Who

- **Ana Lopez** — same as prior stories.
- **Context:** Long-running Services subject to wedge-style failures.
- **Motivation:** Auto-recovery from wedged backends without manual intervention.

### Solution

Extend TOML parser to accept `[[health_check.liveness]]` with the same probe-body shape AND an additional `failure_threshold: u32` (default 3). The `ServiceLifecycleReconciler` tracks consecutive Fail count per liveness probe in the reconciler View (per `.claude/rules/development.md` § "Persist inputs, not derived state" — the counter is an input; the threshold is the policy). When `consecutive_failures >= failure_threshold`, emit `Action::RestartAllocation { alloc_id, reason: LivenessExhausted { probe_idx, consecutive_failures, threshold } }`. Reuses existing JobLifecycle restart machinery (RESTART_BACKOFFS, backoff_for_attempt). Restart counts toward the existing restart_budget (Slice 05 does NOT introduce a separate liveness restart budget).

### Domain Examples

#### 1: Happy path — liveness probe fails 3x consecutively, alloc restarts
Backend's `/healthz` starts returning 503 at T0. With `interval=2s, failure_threshold=3`: Fails at T0, T0+2, T0+4. At reconciler tick after T0+4 fail row, emit `RestartAllocation`. Existing JobLifecycle pathway kills + respawns. `restart_count` increments. New alloc reads probe results from scratch.

#### 2: Edge — liveness recovers within threshold (no restart)
Liveness fails at T0 and T0+2 (consecutive_failures = 2). At T0+4 the probe Passes. consecutive_failures resets to 0. No restart.

#### 3: Error — liveness restart exhausts the restart budget
After RESTART_BACKOFF_CEILING (5) restarts via liveness, JobLifecycle's existing `BackoffExhausted` path fires — wire reports `Failed { reason: BackoffExhausted { attempts: 5 } }` exactly as today.

### UAT Scenarios (BDD)

#### Scenario: Liveness 3 consecutive fails triggers restart
```gherkin
Given a Service alloc 'alloc-payments-0' with a liveness HTTP probe and failure_threshold = 3
And the alloc has restart_count = 0
And the liveness probe fails 3 consecutive ticks
When the next reconciler tick fires
Then an Action::RestartAllocation { alloc_id: alloc-payments-0, reason: LivenessExhausted { ... } } is emitted
And within 1 further tick the AllocStatusRow shows restart_count = 1
```

#### Scenario: Liveness recovery before threshold resets counter
```gherkin
Given a liveness probe with failure_threshold = 3
When liveness fails twice consecutively then Passes
Then no restart action is emitted
And the reconciler View's consecutive_failures counter is 0
```

#### Scenario: Liveness restart consumes the restart_budget
```gherkin
Given a Service alloc with restart_count = 4 and RESTART_BACKOFF_CEILING = 5
When liveness fires its 3-consecutive-fail trigger
Then a fifth restart is dispatched
And on the next subsequent restart trigger, the wire emits Failed { reason: BackoffExhausted { attempts: 5 } }
```

#### Scenario: Liveness probe is Service-kind only
```gherkin
Given a TOML containing `[job]` AND `[[health_check.liveness]]`
When the parser processes it
Then ParseError::ProbesNotAllowedOnKind { kind: "job", ... } is returned
```

### Acceptance Criteria

- [ ] Liveness probe TOML shape parses with same body + `failure_threshold: u32` (default 3)
- [ ] Reconciler View persists `consecutive_failures_per_probe: BTreeMap<ProbeIdx, u32>` (inputs, not derived state)
- [ ] Restart action carries `reason: LivenessExhausted { probe_idx, consecutive_failures, threshold }`
- [ ] Restart consumes the existing restart_budget; eventual BackoffExhausted preserved
- [ ] Liveness probes on Job / Schedule rejected at parse time (covered by US-07)

### Outcome KPIs

K3 — see `outcome-kpis.md`.

### Technical Notes

- Restart action shape: extend existing `Action::RestartAllocation`'s reason field; SemVer additive on a `#[non_exhaustive]` enum.
- Per `BTreeMap` not `HashMap` (per `.claude/rules/development.md` § "Ordered-collection choice").

### Dependencies

- US-04 (proves the per-backend continuous-probe pathway works).
- US-01.

---

## US-06: Operator inspects probe state via `alloc status --job <service-id>`

### Elevator Pitch

- **Before:** Ana has no way to inspect per-probe status without reading server logs. `alloc status` currently shows alloc state only.
- **After:** Ana runs `overdrive alloc status --job payments` and sees a "Probes:" section listing every probe with `role`, `probe_idx`, `mechanic`, `last_status` (ok/fail/pending), `last_observed_at`, and (if Fail) `last_fail_reason`.
- **Decision enabled:** Ana decides whether to debug the workload, tune the probe spec, or restart manually, based on a single-command view of current health.

### Problem

After US-01 lands, Ana knows whether her Service reached Stable, but she has no view into per-probe outcomes after that. If a readiness probe (US-04) is flapping, or a liveness probe (US-05) is approaching its threshold, she has no way to see it short of reading log files. Operator visibility is incomplete.

### Who

- **Ana Lopez** — same as prior stories. Primary `alloc status` consumer.
- **Omar** — future operator using same CLI.
- **Context:** Day-2 operations of a running Service.
- **Motivation:** Self-serve diagnostic without server log access.

### Solution

Extend `crates/overdrive-cli/src/render.rs` Service-kind handler to emit a Probes section per ADR-0033 enrichment. Read `ProbeResultRow`s from the snapshot fetched by the existing alloc-status HTTP handler. Render one row per probe in a table aligned with existing Service render style. Section is ABSENT for Job and Schedule kinds (renderer-side guard).

### Domain Examples

#### 1: Happy path
Stable Service with HTTP startup + HTTP readiness + HTTP liveness, all currently Pass.
```
Service: payments
  spec_digest:  sha256:abcd...
  replicas:     1/1 stable
  stable_since: 2026-05-24T18:42:13Z

Allocations:
  alloc-payments-0   state=Running   terminal=Stable
    Probes:
      startup   #0  http GET /healthz       last=ok    at 18:42:11Z
      readiness #0  http GET /healthz       last=ok    at 18:42:43Z
      liveness  #0  http GET /healthz       last=ok    at 18:42:43Z
```

#### 2: Edge — pending probe (not yet ticked)
Service just submitted; probe row absent yet. Render shows `last=pending` (not blank).

#### 3: Failing probe with reason rendered
```
    Probes:
      startup   #0  tcp 0.0.0.0:8080        last=ok    at 18:42:11Z
      readiness #0  http GET /healthz       last=fail  at 18:43:01Z  HTTP 503
      liveness  #0  http GET /healthz       last=fail  at 18:43:01Z  HTTP 503  (2/3 consecutive fails)
```

### UAT Scenarios (BDD)

#### Scenario: Service with probes renders Probes section
```gherkin
Given a stable Service 'payments' with startup, readiness, and liveness probes
When Ana runs `overdrive alloc status --job payments`
Then the output contains a line "Probes:" indented under the alloc
And the section contains one row per probe with role, probe_idx, mechanic summary, last status, last observed timestamp
```

#### Scenario: Job kind does NOT render Probes section
```gherkin
Given a Job 'cron-cleanup' that has run to Completion
When Ana runs `overdrive alloc status --job cron-cleanup`
Then the output does NOT contain "Probes:" anywhere
```

#### Scenario: Schedule kind does NOT render Probes section
```gherkin
Given a Schedule 'nightly-backup' registered with deferral URL
When Ana runs `overdrive alloc status --job nightly-backup`
Then the output does NOT contain "Probes:"
```

#### Scenario: Probe Fail row renders last_fail_reason
```gherkin
Given a Service with a readiness probe whose latest ProbeResultRow has status Fail and last_fail_reason "HTTP 503"
When Ana runs `overdrive alloc status --job payments`
Then the probe's row contains the substring "HTTP 503"
```

#### Scenario: Just-started Service shows last=pending
```gherkin
Given a Service alloc with probes declared but no ProbeResultRow yet written
When Ana runs `overdrive alloc status --job payments`
Then each probe row contains "last=pending"
```

### Acceptance Criteria

- [ ] Probes section present iff `kind == Service AND probes_present`
- [ ] One row per declared/inferred probe
- [ ] Rows: `role`, `probe_idx`, `mechanic summary` (tcp host:port | http METHOD url | exec command[0]), `last=` status, `at <ISO-8601>`, optional `last_fail_reason`
- [ ] Inferred default probe marked with `(inferred)`
- [ ] Pending state rendered as `last=pending` (not blank)
- [ ] NO_COLOR env var respected; color (red for fail, green for ok) is supplementary not load-bearing

### Outcome KPIs

K4 — see `outcome-kpis.md`.

### Technical Notes

- Pure render-layer change; reads from existing `alloc_status` HTTP handler's snapshot per ADR-0033.
- Snapshot tests in `crates/overdrive-cli/tests/integration/render_probes_section.rs` using `insta` (existing snapshot library).

### Dependencies

- US-01 (ProbeResultRow exists), US-02 / US-03 (renders all mechanic types), US-04 / US-05 (renders all roles).

---

## US-07: Operator who declares probes on Job/Schedule gets a parse-time error naming the right primitive

### Elevator Pitch

- **Before:** Ana, used to k8s where every kind supports probes, drops `[[health_check.startup]]` under `[job]` in her TOML. Today the TOML deserialiser silently accepts it as an unknown field OR fails with a generic shape-mismatch error.
- **After:** Ana sees `Error: probes not allowed on workload kind 'job': Job has no readiness question; on completion is enough.` immediately at parse time, before the spec ever touches the IntentStore.
- **Decision enabled:** Ana stops trying to use probes on the wrong kind and (for Job) learns that exit code IS the success criterion.

### Problem

ADR-0047 establishes per-kind streaming protocols; per-kind probe semantics are similarly closed. A Job is run-to-completion — it has no "ready?" question because its outcome IS its terminal exit code. A Schedule composes per-fire from its `[job]`; probes on the Schedule layer would be semantically nonsense. But without an explicit parser-side rejection, operators will paste probe sections from k8s manifests into the wrong kinds and waste 15 minutes wondering why nothing works.

### Who

- **Ana Lopez** — same.
- **Omar** — future operator, more likely to make this mistake.
- **Context:** Authoring a TOML spec.
- **Motivation:** Get a useful error at parse time, not a silent ignore.

### Solution

Extend the TOML parser (per ADR-0047 §2) to reject `[[health_check.*]]` sections when the outer block is `[job]` or `[schedule]`. Error type `ParseError::ProbesNotAllowedOnKind { kind, guidance }` with kind-specific guidance:
- For `kind = job`: "Job has no readiness question; on completion is enough. Use exit code 0 to indicate success."
- For `kind = schedule`: "Schedule composes per-fire from its [job]; probes on Schedule are semantically meaningless. Probes on the underlying [job] are also rejected — Schedule cannot carry probes at all."

### Domain Examples

#### 1: Job with startup probe rejected
TOML:
```toml
[job]
id = "cron-cleanup"
[[health_check.startup]]
type = "tcp"
port = 9999
```
CLI output:
```
Error: probes not allowed on workload kind 'job'

  Job has no readiness question; on completion is enough.
  Use exit code 0 to indicate success.

  Remove the [[health_check.*]] sections from your spec, or change
  [job] to [service] if this workload is intended to be long-lived.
```

#### 2: Schedule with liveness probe rejected
TOML:
```toml
[schedule]
cron = "0 * * * *"
[job]
id = "hourly"
[[health_check.liveness]]
type = "http"
path = "/health"
port = 8080
```
CLI prints the schedule-specific guidance.

#### 3: Service with probes accepted (control)
A `[service]` block with `[[health_check.startup]]` parses without error (covered by US-02 acceptance).

### UAT Scenarios (BDD)

#### Scenario: Job with any probe section is rejected with named guidance
```gherkin
Given a TOML containing a [job] block and at least one [[health_check.startup|readiness|liveness]] section
When Ana runs `overdrive job submit bad.toml`
Then the CLI prints "Error: probes not allowed on workload kind 'job'"
And the output contains the guidance "Job has no readiness question; on completion is enough."
And the exit code is 1
And no IntentStore write occurs
```

#### Scenario: Schedule with any probe section is rejected with schedule-specific guidance
```gherkin
Given a TOML containing a [schedule] block and at least one [[health_check.*]] section
When Ana runs `overdrive job submit bad.toml`
Then the CLI prints "Error: probes not allowed on workload kind 'schedule'"
And the output contains "Schedule composes per-fire"
And the exit code is 1
```

#### Scenario: Service with probe sections is accepted (regression guard)
```gherkin
Given a TOML containing a [service] block and at least one [[health_check.startup]] section
When Ana runs `overdrive job submit good.toml`
Then no parse error is raised
And the Accepted line is printed
```

### Acceptance Criteria

- [ ] Job + any `[[health_check.*]]` → `ParseError::ProbesNotAllowedOnKind { kind: "job", guidance: "Job has no readiness question; on completion is enough." }`
- [ ] Schedule + any `[[health_check.*]]` → `ParseError::ProbesNotAllowedOnKind { kind: "schedule", guidance: "Schedule composes per-fire ..." }`
- [ ] Service + any `[[health_check.*]]` → no error (regression)
- [ ] CLI exit code 1 for the reject cases; 0 (or stream-continues) for the accept case
- [ ] Error never reaches IntentStore (parse-time guard)

### Outcome KPIs

K5 — see `outcome-kpis.md`.

### Technical Notes

- Independent of US-01..06; can land in parallel.
- Pure parser-side change; no runtime impact.
- `ParseError` enum is already established in `overdrive-core`; this adds one variant.

### Dependencies

- ADR-0047 (per-kind discriminator) landed.

---

## US-08: Service workload that exits within startup deadline emits EarlyExit (closes RCA-A coinflip case)

### Elevator Pitch

- **Before:** Ana submits a Service spec whose entrypoint exits 1 within milliseconds (port collision, missing env var, immediate panic). CLI reports `is running (took live)` exit 0 — the original RCA-A failure mode.
- **After:** CLI prints `Service '<id>' failed: workload exited within startup deadline\n  exit_code: 1\n  elapsed: 0.05s (startup_deadline=60s)\n  stderr_tail: "..."` and exits 1.
- **Decision enabled:** Ana sees the exit code and stderr tail; debugs the spec rather than wasting time wondering if the platform is broken.

### Problem

This is RCA root cause A applied specifically to short-lived Services. US-01 establishes that the EarlyExit variant exists and fires; this story locks in the operator-facing rendering details, the stderr_tail capture, and the integration-test-as-RCA-regression-guard. The story is sized as a separate slice because the rendering and ExitObserver-integration concerns are crisply scoped.

### Who

- **Ana Lopez** — bitten by the RCA on 2026-05-09.
- **Context:** Iterating on a Service spec; entrypoint has a port collision or panics on init.
- **Motivation:** See WHY the workload died, not "it's running".

### Solution

The `ServiceLifecycleReconciler` (from US-01) treats an `AllocStatusRow { state: Failed, exit_code }` row written within `startup_deadline` AND with NO Pass ProbeResultRow as the trigger for `Failed { reason: EarlyExit { exit_code } }`. The stderr_tail is captured by the existing ExitObserver (per `crates/overdrive-control-plane/src/worker/exit_observer.rs`) — this story DOES NOT introduce new stderr capture, only ensures the captured value flows into the wire event payload AND the CLI render. The streaming `ServiceSubmitEvent::Failed { reason: EarlyExit { exit_code }, stderr_tail }` is the wire shape; CLI renders it with the multi-line format shown above.

### Domain Examples

#### 1: Port collision exit 1 at T0+30ms
Workload binds 0.0.0.0:8080, port already in use, exits 1. EarlyExit emitted with stderr_tail showing the bind error.

#### 2: Missing env var, exit 2 at T0+50ms (custom non-zero)
EarlyExit { exit_code: 2 }; CLI render shows exit_code: 2 (any non-zero is failure for Service kind).

#### 3: Edge — exit 0 within startup_deadline (also failure for Service kind)
A Service whose entrypoint completes successfully and exits 0 immediately is STILL a Service failure (Service expects long-lived). EarlyExit { exit_code: 0 } emitted; CLI render shows "workload exited with code 0 within startup deadline" — to distinguish, the render's text explains "Service kind expects long-lived; use [job] for run-to-completion."

### UAT Scenarios (BDD)

#### Scenario: Service exit 1 within startup_deadline emits EarlyExit with exit_code
```gherkin
Given a Service spec whose [exec] command exits 1 within 100ms
And the alloc has no Pass ProbeResultRow yet
And startup_deadline has NOT elapsed
When the ServiceLifecycleReconciler ticks after the ExitObserver writes the Failed row
Then it emits Action::SetTerminalCondition { Failed { reason: EarlyExit { exit_code: 1 } } }
And the wire event is ServiceSubmitEvent::Failed { reason: EarlyExit { exit_code: 1 }, stderr_tail: "..." }
```

#### Scenario: stderr_tail flows from ExitObserver capture to wire event to CLI render
```gherkin
Given a Service whose command writes "ERROR: failed to bind 0.0.0.0:8080" to stderr then exits 1 at T0+50ms
When the EarlyExit wire event is emitted
Then the stderr_tail field contains the substring "failed to bind"
And the CLI render line "  stderr_tail:" contains the same substring
```

#### Scenario: Service exit 0 within deadline is still EarlyExit failure (long-lived expectation)
```gherkin
Given a Service spec whose [exec] is `/bin/true` (exits 0 immediately)
When the reconciler observes the terminal Failed row
Then EarlyExit { exit_code: 0 } is emitted (not Stable)
And the CLI render explains "Service kind expects long-lived; use [job] for run-to-completion"
And the exit code is 1
```

#### Scenario: Exit after startup_deadline + Stable is NOT EarlyExit
```gherkin
Given a Service that reached Stable at T0+5s
And the workload subsequently exits at T0+10s
When the reconciler observes the Failed row at T0+10s
Then the wire event is NOT EarlyExit
And the wire event is liveness-driven restart (US-05) or BackoffExhausted (existing)
```

#### Scenario: Coinflip-reshaped-as-Service regression test passes
```gherkin
Given the test fixture `examples/coinflip-as-service.toml` (coinflip's exec with [service] block and a listener)
When the integration test runs 100 deterministic seeds of submit
Then at least 99 of 100 submissions emit ServiceSubmitEvent::Failed { reason: EarlyExit { ... } }
And zero submissions emit ServiceSubmitEvent::Stable
And the CLI never prints "(took live)"
```

### Acceptance Criteria

- [ ] EarlyExit triggered when AllocStatusRow.state == Failed AND no Pass ProbeResultRow AND elapsed < startup_deadline
- [ ] stderr_tail flows from existing ExitObserver capture to wire event to CLI render — no new capture introduced
- [ ] Service exit 0 within startup_deadline = EarlyExit { exit_code: 0 } (NOT Stable)
- [ ] Exit AFTER Stable is NOT EarlyExit (existing restart / BackoffExhausted paths apply)
- [ ] Regression test `service_honest_stable.rs` covers the coinflip-shaped fixture; ≥99/100 deterministic seeds emit Failed (closes K1)

### Outcome KPIs

K1 — see `outcome-kpis.md`. This story is the direct closing of RCA-A for Service kind.

### Technical Notes

- Reuses existing ExitObserver — does not modify it.
- `EarlyExit { exit_code }` variant on the new `ServiceFailureReason` enum (alongside `StartupProbeFailed { ... }` from US-01).

### Dependencies

- US-01 (Stable/Failed wire variants; reconciler split).

---

## Anti-pattern remediation log

The following anti-patterns were detected during drafting and remediated in-place:

| Pattern | Where | Remediation |
|---|---|---|
| "Implement probes" | early draft of US-01 | Rewrote with Ana's 2026-05-09 RCA experience as the pain point |
| Generic data ("user123") | n/a (always real names) | — |
| Technical AC ("Use HTTP client X") | early draft of US-02 | Replaced with "HTTP 2xx within timeout = Pass" — observable outcome |
| Oversized story (probe types + roles + render in one story) | original sizing | Split into 8 slices — see story-map.md carpaccio analysis |
| Implementation-y scenario titles | n/a | Used "Service reaches Stable" / "Service fails" patterns |

## Glossary (ubiquitous language)

| Term | Meaning |
|---|---|
| **Probe** | A single declared or inferred health check |
| **Role** | `startup` / `readiness` / `liveness` |
| **Mechanic** | `http` / `tcp` / `exec` |
| **probe_idx** | 0-indexed position in the TOML array (or 0 for inferred default) |
| **Stable** | TerminalCondition: Service has reached operator-meaningful liveness |
| **EarlyExit** | Failure reason: workload exited before any startup probe could pass |
| **StartupProbeFailed** | Failure reason: startup probe never passed within deadline |
| **settled_in** | Real Duration from `started_at` to deciding tick — never the literal `"live"` |
| **witness** | The probe (probe_idx + role) whose Pass moved the reconciler to Stable |
| **startup_deadline** | Computed: `timeout × max_attempts` summed across startup probes; default 60s |
