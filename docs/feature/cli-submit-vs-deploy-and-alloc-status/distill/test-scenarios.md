# Test Scenarios — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DISTILL (acceptance-designer / Quinn)
**Date**: 2026-04-30
**Status**: ready for reviewer (`nw-acceptance-designer-reviewer`).

This catalogue is the SPECIFICATION the crafter translates into Rust
`#[test]` / `#[tokio::test]` bodies during DELIVER. Per
`.claude/rules/testing.md`, **no `.feature` files exist anywhere in
this codebase** — Gherkin GIVEN/WHEN/THEN blocks are specification-only
and live exclusively in this document. The crafter writes:

- **Tier 1 (DST + acceptance)** scenarios as Rust `#[test]` /
  `#[tokio::test]` files at
  `crates/overdrive-control-plane/tests/acceptance/<scenario>.rs`,
  wired through `crates/overdrive-control-plane/tests/acceptance.rs`.
- **Tier 3 (real-kernel integration)** scenarios as Rust
  `#[tokio::test]` files at
  `crates/overdrive-control-plane/tests/integration/<scenario>.rs`,
  wired through `crates/overdrive-control-plane/tests/integration.rs`
  under the `integration-tests` feature flag, with
  `#[cfg(target_os = "linux")]` per-file gate where the scenario
  requires real kernel I/O.

CLI-side scenarios live under
`crates/overdrive-cli/tests/acceptance/<scenario>.rs` and
`crates/overdrive-cli/tests/integration/<scenario>.rs` respectively.

The four-tier model from `.claude/rules/testing.md` applies. **Tiers
2 (BPF unit) and 4 (verifier / perf) are N/A for this feature** — no
eBPF programs are touched.

---

## 1. Coverage map (story → scenario)

Every AC bullet from `discuss/user-stories.md` and every KPI from
`discuss/outcome-kpis.md` maps to at least one named scenario below.

| Story / KPI | AC bullet | Scenario(s) | Tier |
|---|---|---|---|
| US-01 | streaming submit on healthy spec exits 0 with summary | S-WS-01, S-CP-01 | T3, T1 |
| US-01 | first NDJSON line lands within 200 ms p95 | S-CP-02 | T1 |
| US-01 | re-submit unchanged spec exits 0 with `outcome: Unchanged` first event | S-CP-03 | T1 |
| US-01 | lifecycle transitions stream as NDJSON lines | S-CP-04 | T1 |
| US-02 | broken-binary spec exits 1 with structured `Error:` block | S-WS-02, S-CLI-04 | T3, T1 |
| US-02 | verbatim driver error appears in CLI output | S-WS-02, S-CP-05 | T3, T1 |
| US-02 | CLI output names a reproducer command | S-CLI-04 | T1 |
| US-02 | server wall-clock cap exceeded → `ConvergedFailed { Timeout }`; exit 1 | S-CP-06 | T1 |
| US-02 | `reason` byte-equal across stream and snapshot | S-WS-02, S-CP-07 | T3, T1 |
| US-03 | `--detach` sends `Accept: application/json` regardless of TTY | S-CLI-01, S-CP-08 | T1 |
| US-03 | output is a single JSON object equivalent to today's shape | S-CLI-01 | T1 |
| US-03 | exit 0 on successful commit | S-CLI-01 | T1 |
| US-03 | exit 2 on transport / server-validation error | S-CLI-05 | T1 |
| US-04 | CLI calls `IsTerminal` and sends `application/json` when not a TTY | S-CLI-02 | T1 |
| US-04 | `submit | jq -r .spec_digest` works without `--detach` | S-CLI-03 | T3 |
| US-04 | `submit > file.json` produces a single JSON object | S-CLI-02 | T1 |
| US-04 | TTY without `--detach` defaults to NDJSON streaming | S-CLI-06 | T1 |
| US-05 | snapshot wire shape carries 6+ fields | S-AS-01, S-AS-02 | T1 |
| US-05 | CLI renders journey TUI mockup for Running and Failed | S-AS-04, S-AS-05 | T1 |
| US-05 | verbatim driver error in Failed-case rendering | S-AS-05 | T1 |
| US-05 | `(backoff exhausted)` annotation when applicable | S-AS-05, S-AS-08 | T1 |
| US-05 | Pending-no-capacity renders explicit reason, not silent zero | S-AS-06 | T1 |
| US-06 | one source of truth for `transition_reason` per allocation | S-CP-07, S-AS-07 | T1 |
| US-06 | streaming `LifecycleTransition.reason` == snapshot `last_transition.reason` | S-CP-07, S-WS-02 | T1, T3 |
| US-06 | streaming `ConvergedFailed.error` == snapshot per-row `error` | S-WS-02 | T3 |
| US-06 | integration test asserts byte-for-byte equality on broken-binary | S-WS-02 | T3 |
| KPI-01 | first-event ≤ 200 ms p95 | S-CP-02 | T1 |
| KPI-02 | broken-binary surfaces failure inline (boolean) | S-WS-02 | T3 |
| KPI-03 | snapshot field count ≥ 6 | S-AS-01 | T1 |
| KPI-04 | byte-for-byte equality across surfaces | S-WS-02, S-CP-07 | T3, T1 |
| KPI-05 | `--detach` exit ≤ 200 ms p95 | S-CLI-01 | T1 |

Tier legend: **T1** = DST in-process (acceptance suite); **T3** =
real-kernel integration (linux-gated, `integration-tests` feature).

`S-WS-*` = walking-skeleton-class scenarios (driving-adapter
verification through real protocol). `S-CP-*` = control-plane handler
scenarios. `S-CLI-*` = CLI-side scenarios. `S-AS-*` = `alloc status`
snapshot scenarios.

---

## 2. Adapter coverage table (Mandate 6)

For each driven adapter touched by this feature, the table records
whether it has a Tier-3 `@real-io` scenario or is covered by the Sim
trait at Tier 1.

| Driven adapter | Coverage | Scenario | Rationale |
|---|---|---|---|
| `IntentStore` (real `LocalIntentStore` over redb) | EXISTING T3 + new T1 | S-WS-01 (existing), S-CP-03 idempotency | `LocalIntentStore` is already covered by `concurrent_submit_toctou`, `idempotent_resubmit`, `submit_round_trip`. The new write paths (none) require no new T3 — the existing coverage extends. |
| `ObservationStore` (real `LocalObservationStore` *or* `SimObservationStore`) | NEW T1 + indirect T3 | S-CP-09 (row schema additive), S-WS-01/S-WS-02 (live row reads under integration) | `AllocStatusRow.reason` and `.detail` are NEW additive fields per ADR-0032 §4. Tier 1 round-trip proptest (`SubmitEvent` ↔ row ↔ snapshot) verifies serde + rkyv. Tier 3 `S-WS-01` exercises the real backend implicitly. |
| `Driver` trait — real `ExecDriver` | NEW T3 (broken-binary) + EXTENDS existing T3 | S-WS-02 (broken-binary regression), S-WS-01 (happy path) | Existing `submit_to_running` covers happy path with `/bin/sleep`. `S-WS-02` is the **regression-target** for ENOENT — must invoke real `tokio::process::Command::spawn` with a non-existent binary path. `SimDriver` cannot catch the wiring of ENOENT → `DriverError::StartRejected.reason` → `AllocStatusRow.detail`. |
| Broadcast channel (`tokio::sync::broadcast::Sender<LifecycleEvent>`) | NEW T1 only | S-CP-04, S-CP-05, S-CP-06 | In-process channel; deterministic under tokio runtime. Lagged-recovery synthesised in S-CP-10. |
| `Clock` trait — `SystemClock` / `SimClock` for the cap timer | NEW T1 only | S-CP-06 | `SimClock` advances simulated time past `WALL_CLOCK_CAP`; the production `SystemClock` path is identical (per ADR-0013 §2c). |
| HTTP transport (axum router, `application/x-ndjson` content negotiation, NDJSON line emission) | NEW T1 (oneshot router) + NEW T3 (real reqwest stream) | S-CP-01 (router oneshot), S-WS-01 (real reqwest streaming) | `axum::ServiceExt::oneshot` exercises the handler in-process at Tier 1. The driving-adapter-verification gate requires Tier 3 with real reqwest against a bound port — `S-WS-01` covers this. |
| CLI subprocess (driving adapter) | NEW T3 only | S-WS-01 (happy path), S-WS-02 (broken binary) | Per the driving-adapter mandate, the CLI must be invoked via real `Command::new("overdrive")` against a real spawned control-plane process. Tier 1 cannot catch arg-parsing, `IsTerminal` interaction with the subprocess's stdout, or exit-code propagation. |

**Audit result**: every adapter has a defined coverage line. Zero
"NO — MISSING" rows.

---

## 3. Scenarios

### 3.1 Driving-adapter walking-skeleton scenarios (T3, real-kernel)

The DESIGN names two driving adapters: the **CLI subprocess** and the
**HTTP API**. Per the driving-adapter verification mandate, at least
one Tier-3 scenario per driving adapter must invoke it via its real
protocol. Both `S-WS-01` and `S-WS-02` exercise both driving adapters
together (the CLI invokes the HTTP API), satisfying both gates with
two scenarios.

DISCUSS [D8] / DESIGN [C7] **waived** the formal walking-skeleton
artifact for this brownfield extension; nonetheless the driving-adapter
verification gate REQUIRES at least one end-to-end scenario per driving
adapter. Both `S-WS-*` scenarios are tagged `@walking_skeleton
@driving_adapter @real-io` for catalogue traceability — they are the
load-bearing structural-end-to-end coverage.

#### S-WS-01 — Operator submits a healthy spec and the verb tells the truth on success

Tier 3 — Linux-only. Invokes the real `overdrive` CLI binary as a
subprocess against a real spawned control-plane process. The CLI
stream-consumes the NDJSON response from the real HTTP transport.

```gherkin
Scenario: Operator submits a healthy spec and the verb tells the truth on success
  Given a control plane is running with the lifecycle reconciler registered
  And a job spec declares a binary that exists and exits cleanly
  When the operator runs `overdrive job submit ./payments.toml` from a TTY
  Then the CLI receives an `Accepted` event carrying the spec digest
  And the CLI receives one or more lifecycle transition events
  And the CLI receives a terminal `running` confirmation event
  And the CLI prints a summary line naming the job and replica count
  And the CLI exits with status 0
```

**Driving port**: real subprocess `Command::new(overdrive_bin).args(["job",
"submit", &toml_path])` + real HTTP via the spawned control plane's
bound port.

**Sim/real adapter substitutions**: NONE — real `LocalIntentStore`,
real `LocalObservationStore`, real `ExecDriver` against a real
`/bin/sleep`-class binary. `SystemClock` (the test does not race the
60s cap; convergence completes in seconds).

**Driving adapters exercised**:
1. CLI subprocess (real `tokio::process::Command::spawn` of the
   `overdrive` binary, real arg parsing, real `IsTerminal` against the
   pty the test allocates, real exit code).
2. HTTP API (real reqwest streaming over `application/x-ndjson` against
   the bound port).

**Lima VM gate**: when run on macOS, the test must execute via
`cargo xtask lima run --` per `.claude/rules/testing.md` § "Running
integration tests locally on macOS — Lima VM". On Linux runners, no
prefix required.

**Asserts**: exit code is 0; stdout contains a summary line of the
shape `Job '<name>' is running with 1/1 replicas (took ...)`; stdout
contains at least one `LifecycleTransition`-shaped NDJSON line and one
`ConvergedRunning`-shaped NDJSON line; the CLI's NDJSON parse does not
hang.

#### S-WS-02 — Operator submits a broken-binary spec and the verb names the cause (REGRESSION TARGET)

Tier 3 — Linux-only. **The load-bearing scenario for the entire
feature.** Invokes the real `overdrive` CLI subprocess against a real
spawned control-plane process. The job spec points at
`/usr/local/bin/no-such-binary`. The real `ExecDriver` returns ENOENT
through `tokio::process::Command::spawn`. The lifecycle reconciler
exhausts its restart budget. The streaming endpoint emits
`ConvergedFailed`. The CLI exits 1 with the verbatim driver error in
its output.

```gherkin
Scenario: Operator submits a broken-binary spec and the verb names the cause
  Given a control plane is running with the lifecycle reconciler registered
  And a job spec declares a binary path that does not exist on the host
  When the operator runs `overdrive job submit ./payments.toml` from a TTY
  Then the CLI receives an `Accepted` event
  And the CLI receives lifecycle transition events naming a `driver start failed` reason
  And the verbatim driver error text appears in the CLI's output
  And the CLI receives a terminal `failed` event with reason `backoff exhausted`
  And the CLI prints an `Error:` block naming the reason, the verbatim error, and a reproducer command
  And the CLI exits with status 1
  And when the operator subsequently runs `overdrive alloc status --job <id>` the snapshot's `last_transition.reason` byte-equals the streaming reason
  And the snapshot's per-row `error` byte-equals the streaming `error`
```

**Driving port**: same as `S-WS-01` plus a second subprocess invocation
of `overdrive alloc status`. Both subprocess invocations parse real
arguments and produce real exit codes.

**Sim/real adapter substitutions**: NONE for the runtime path —
real `LocalIntentStore`, real `LocalObservationStore`, real
`ExecDriver`. The `WALL_CLOCK_CAP` is **not** raced (5 attempts × 5s
backoff = 25s, well under the 60s default cap; convergence to Failed
happens on backoff-exhausted, not on cap).

**Driving adapters exercised**: CLI subprocess + real HTTP API
(streaming AND snapshot — two separate invocations of `overdrive`).

**Lima VM gate**: macOS runs via `cargo xtask lima run --`. CI runs on
Linux without prefix.

**Asserts**: CLI exit code is 1; CLI stdout (or stderr — TBD by the
crafter per render contract) contains the literal substring `stat
/usr/local/bin/no-such-binary: no such file or directory` (or whatever
the real ENOENT formatter emits — assertion is "substring of the live
syscall error"); CLI stdout contains the substring "reproducer";
streaming `ConvergedFailed.terminal_reason` is `backoff_exhausted`;
streaming `ConvergedFailed.error` byte-equals the snapshot's per-row
`error`; streaming `LifecycleTransition.reason` (the last
`driver_start_failed` event) byte-equals the snapshot's
`last_transition.reason`.

**KPI binding**: KPI-02 (broken-binary surfaces failure inline) — this
scenario IS the boolean test for the load-bearing KPI. KPI-04
(byte-for-byte equality) — the cross-surface assertion lives here.

**Why this scenario cannot be moved to Tier 1**: `SimDriver` returning
a fabricated `DriverError::StartRejected` does not catch the wiring
between `tokio::process::Command::spawn(\"/usr/local/bin/no-such-binary\")`
and `DriverError::StartRejected.reason`. The structural-end-to-end
property is "real ENOENT propagates through real driver into real
broadcast channel into real NDJSON serialisation into real CLI exit
code." Tier 1 misses every wiring bug in that chain.

---

### 3.2 Streaming-submit control-plane scenarios (T1, DST in-process)

These exercise the streaming-submit handler logic in isolation via
`axum::ServiceExt::oneshot` against a router built with sim adapters.
Run in the default unit lane — no real I/O.

#### S-CP-01 — Streaming submit emits Accepted, LifecycleTransition, and ConvergedRunning lines on a healthy convergence

```gherkin
Scenario: Streaming submit emits Accepted, LifecycleTransition, and ConvergedRunning lines on a healthy convergence
  Given the streaming-submit handler is wired with a sim driver returning Ok(handle)
  And the lifecycle reconciler converges the allocation to running
  When a request arrives with `Accept: application/x-ndjson`
  Then the response is one NDJSON line per event
  And the first line is `Accepted` carrying spec_digest, intent_key, and outcome
  And subsequent lines are `LifecycleTransition` events with structured reason and source
  And the terminal line is `ConvergedRunning` carrying alloc_id and started_at
  And the response stream closes after the terminal line
```

**Driving port**: `axum::Router::oneshot(Request::post("/v1/jobs"))`
on the production router with sim adapters in `AppState`.

**Sim/real adapter substitutions**: `SimDriver` returning `Ok(handle)`,
`SimObservationStore`, `SimClock`, `LocalIntentStore` over a tempdir
redb (acceptable in default lane — fast, isolated). The broadcast
channel is a real `tokio::sync::broadcast::Sender`.

**Asserts**: response `Content-Type` is `application/x-ndjson`;
response status is `200 OK`; the body, line-split, parses to a sequence
beginning with `SubmitEvent::Accepted` and ending with
`SubmitEvent::ConvergedRunning`; every line is valid JSON; every line
carries a `kind` discriminator.

#### S-CP-02 — First NDJSON line is delivered within 200 ms p95 (KPI-01)

```gherkin
Scenario: First NDJSON line is delivered within 200 ms p95
  Given the streaming-submit handler is wired with sim adapters
  And the IntentStore commit returns Inserted within 50 ms
  When a request arrives with `Accept: application/x-ndjson`
  Then the first NDJSON line is on the wire within 200 ms of the request hitting the handler
```

**Driving port**: `axum::Router::oneshot` with timing instrumentation
around the request future.

**Sim/real adapter substitutions**: `SimClock` advanced explicitly;
`SimObservationStore`; `LocalIntentStore` (in-memory mode if
available, else tempdir).

**Asserts**: under DST control of `SimClock`, the wall-clock delta
between request entry and first emitted byte is < 200 ms in 95% of
runs (proptest with 1024 cases). On a real-time `SystemClock` smoke
run inside `S-WS-01`, the same property holds qualitatively.

**KPI binding**: KPI-01 (200 ms first-event p95).

**Property-test shape**: parametrise over IntentStore commit latency
∈ [0 ms, 100 ms]; assert first-line delta ≤ 200 ms in every case the
generator produces.

#### S-CP-03 — Re-submit of an unchanged spec emits Accepted with outcome Unchanged

```gherkin
Scenario: Re-submit of an unchanged spec emits Accepted with outcome Unchanged
  Given a job has already been submitted and committed to the IntentStore
  When the operator re-submits the byte-identical spec via streaming
  Then the first NDJSON line is `Accepted` with `outcome: Unchanged`
  And the stream emits a single `ConvergedRunning` event referencing the existing allocation
  And the stream closes
```

**Driving port**: `axum::Router::oneshot` invoked twice against the
same `LocalIntentStore`.

**Sim/real adapter substitutions**: same as S-CP-01.

**Asserts**: the second invocation's first line is
`SubmitEvent::Accepted { outcome: IdempotencyOutcome::Unchanged, .. }`;
the second invocation's terminal line is
`SubmitEvent::ConvergedRunning { alloc_id, .. }` referencing the
allocation produced by the first invocation; no `LifecycleTransition`
lines appear in the second invocation's body when the allocation is
already Running.

#### S-CP-04 — Each lifecycle transition produces exactly one NDJSON LifecycleTransition line

```gherkin
Scenario: Each lifecycle transition produces exactly one NDJSON LifecycleTransition line
  Given the streaming-submit handler is subscribed to the broadcast channel
  When the action shim writes a sequence of N AllocStatusRow transitions
  Then the streaming response body contains exactly N `LifecycleTransition` lines
  And each line carries from, to, reason, source, and timestamp
```

**Driving port**: `axum::Router::oneshot` plus direct broadcast-send
calls from the test driver acting as the action shim.

**Sim/real adapter substitutions**: real `tokio::sync::broadcast`;
synthetic `LifecycleEvent` payloads constructed in-test (the
test acts as the action shim).

**Asserts**: line count equals event count; line ordering matches
event ordering; every line's `from`/`to` pair matches the broadcast
event's pair.

**Property-test shape**: parametrise N ∈ [1, 32], parametrise the
sequence of state transitions; assert line count and ordering across
1024 generator cases.

#### S-CP-05 — Driver-start-failed transition surfaces verbatim driver text in detail

```gherkin
Scenario: Driver-start-failed transition surfaces verbatim driver text in detail
  Given a sim driver configured to return DriverError::StartRejected with reason text "stat /no/such: no such file or directory"
  When the streaming-submit handler observes the resulting AllocStatusRow
  Then the corresponding NDJSON `LifecycleTransition` line carries `reason: driver_start_failed`
  And the line carries `detail: "stat /no/such: no such file or directory"`
```

**Driving port**: `axum::Router::oneshot` with `SimDriver` configured
to return `StartRejected`.

**Asserts**: the `detail` field on the line is set; the `reason` field
is the `DriverStartFailed` variant; the `source` is
`Driver(DriverType::Exec)`.

**Note**: this is the Tier-1 mirror of `S-WS-02`. T1 catches the
serialisation shape; T3 catches that real ENOENT actually propagates.

#### S-CP-06 — Server wall-clock cap fires when convergence does not complete in time

```gherkin
Scenario: Server wall-clock cap fires when convergence does not complete in time
  Given the streaming-submit handler is configured with a 60-second cap
  And the broadcast channel never delivers a terminal LifecycleEvent
  When the simulated clock advances past 60 seconds
  Then the response body's terminal line is `ConvergedFailed` with `terminal_reason: timeout`
  And the response body's terminal line carries `error: "did not converge in 60s"`
  And the response stream closes after the terminal line
```

**Driving port**: `axum::Router::oneshot` with `SimClock` driving the
cap timer.

**Sim/real adapter substitutions**: `SimClock` advanced via the DST
harness (`turmoil::sim::advance(...)`); broadcast channel held open
with no events sent.

**Asserts**: terminal line is `SubmitEvent::ConvergedFailed`; inner
`terminal_reason` is `TerminalReason::Timeout`; inner `error` is the
exact string `"did not converge in 60s"`; the cap timer fired exactly
once; no `LifecycleTransition` lines appeared between `Accepted` and
the timeout terminal.

**KPI / mandate binding**: structurally enforces ADR-0032 §6 (handler-
local `select!` with injected `Clock`); the DST invariant
`StreamingSubmitTerminalEventBoundedByCap` from
`design/wave-decisions.md` lives here.

#### S-CP-07 — Streaming reason and snapshot reason are the same TransitionReason value (KPI-04, T1 mirror)

```gherkin
Scenario: Streaming reason and snapshot reason are the same TransitionReason value
  Given a streaming submit emits a `LifecycleTransition { reason: <R> }` line for allocation A
  And the action shim writes the corresponding AllocStatusRow with reason field set to <R>
  When the operator runs `alloc status --job <id>` against the same observation store
  Then the snapshot's `last_transition.reason` for allocation A is the same TransitionReason variant <R>
  And both surfaces serialise <R> to the same snake_case wire string
```

**Driving port**: `axum::Router::oneshot` invoked twice — once on
`POST /v1/jobs` (NDJSON), once on `GET /v1/allocs?job=<id>` (JSON).

**Sim/real adapter substitutions**: shared `SimObservationStore`
between the two invocations; real broadcast channel.

**Asserts**: parse the streaming line's `reason` field as a
`TransitionReason` enum value; parse the snapshot's
`last_transition.reason` field as a `TransitionReason` enum value;
assert structural equality (same variant); assert the serialised JSON
strings are byte-equal across the two surfaces.

**Property-test shape**: parametrise the `TransitionReason` variant
over all 8 enum values; assert the round-trip property in every case.

#### S-CP-08 — JSON-lane (Accept: application/json) returns the existing SubmitJobResponse shape unchanged

```gherkin
Scenario: JSON-lane returns the existing SubmitJobResponse shape unchanged
  Given the submit_job handler is wired
  When a request arrives with `Accept: application/json`
  Then the response Content-Type is `application/json`
  And the response body is a single SubmitJobResponse JSON object
  And the body carries spec_digest, intent_key, and outcome fields
  And no NDJSON streaming machinery is engaged
```

**Driving port**: `axum::Router::oneshot` with
`Accept: application/json`.

**Sim/real adapter substitutions**: same as S-CP-01.

**Asserts**: response `Content-Type` is `application/json`; response
body parses as `SubmitJobResponse`; body shape byte-equals what the
JSON-only `submit_job` handler emits in today's pre-feature codebase
(captured in a regression-grade snapshot fixture).

**Mandate binding**: ADR-0032 §1 back-compat surface; DESIGN [C2]
"existing JSON ack shape is RETAINED unchanged."

#### S-CP-09 — AllocStatusRow round-trips through rkyv with the new reason and detail fields

```gherkin
Scenario: AllocStatusRow round-trips through rkyv with the new reason and detail fields
  Given an AllocStatusRow with reason set to a TransitionReason variant and detail set to a string
  When the row is rkyv-archived and deserialised
  Then the round-tripped row equals the original
  And rows with reason None and detail None archive to the same bytes as the pre-feature shape (forward-compat)
```

**Driving port**: direct call to `rkyv::to_bytes` /
`rkyv::access::<ArchivedAllocStatusRow>` on the row type.

**Sim/real adapter substitutions**: NONE — pure proptest on the type.

**Asserts**: round-trip equality; backwards compatibility with
pre-feature serialised bytes.

**Property-test shape**: parametrise over all 8 `TransitionReason`
variants × `Option<String>` detail × every `AllocState` variant
including the new `Failed`; assert bidirectional round-trip across
1024 cases.

**Mandate binding**: ADR-0032 §4 — `AllocStatusRow` rkyv archive shape
is additive and forward-compatible.

#### S-CP-10 — Lagging broadcast subscriber recovers via observation-store snapshot

```gherkin
Scenario: Lagging broadcast subscriber recovers via observation-store snapshot
  Given the streaming-submit handler is subscribed to a broadcast channel of capacity 4
  And 5 LifecycleEvents are pushed before the handler reads any
  When the handler attempts to receive the next event
  Then it observes a Lagged error
  And it recovers by reading the latest AllocStatusRow snapshot from the ObservationStore
  And it synthesises any missing transitions from prior cached state
  And it resubscribes and continues normally
```

**Driving port**: `axum::Router::oneshot` against a deliberately
under-sized broadcast channel.

**Sim/real adapter substitutions**: real broadcast channel; sim
observation store.

**Asserts**: on `RecvError::Lagged(n)`, the handler does not panic;
the response body still emits the correct sequence of
`LifecycleTransition` lines for the post-lag transitions; the response
terminates correctly.

**Note**: defensive scenario per ADR-0032 §7 lagging-discipline. May
be deferred to Phase 2+ if Slice 02's complexity budget is tight —
the crafter MAY mark this `#[ignore]` if it adds complexity but MUST
emit it as a named scaffolded test, not silently drop it.

---

### 3.3 CLI-side scenarios (T1 / T3 split as named)

These exercise CLI behaviour. T1 tests use a fake HTTP server (axum
oneshot served on a bound port within the test process); T3 tests
invoke the real `overdrive` binary against the real spawned control
plane.

#### S-CLI-01 — `--detach` flag sends Accept: application/json regardless of TTY (T1)

```gherkin
Scenario: --detach flag sends Accept: application/json regardless of TTY
  Given the CLI submit command receives the --detach flag
  When the CLI builds the HTTP request
  Then the Accept header is `application/json`
  And the CLI does not engage the NDJSON consumer
  And on a successful response the CLI exits with status 0
```

**Driving port**: the CLI's `submit` command function, invoked
in-process with `--detach` set in the parsed args struct.

**Sim/real adapter substitutions**: in-process axum router as the HTTP
backend; CLI's reqwest client sends to a bound localhost port.

**Asserts**: the request the test backend captures has
`Accept: application/json`; the CLI exit-code accumulator records 0;
CLI stdout is a single JSON object (not a sequence of NDJSON lines).

**KPI binding**: KPI-05 (`--detach` exit ≤ 200 ms p95) — the test
asserts wall-clock from invocation to exit is < 200 ms with a real-
time clock (this is a real-time assertion, not DST-controlled, since
the CLI's reqwest client uses `tokio::time` directly).

#### S-CLI-02 — CLI auto-detaches when stdout is not a TTY (T1)

```gherkin
Scenario: CLI auto-detaches when stdout is not a TTY
  Given the CLI's stdout is redirected to a file
  And the --detach flag is not present
  When the CLI submit command runs
  Then the CLI's IsTerminal probe reports stdout is not a TTY
  And the request's Accept header is `application/json`
  And the file contains a single JSON object after the run
```

**Driving port**: the CLI's `submit` command function, invoked
in-process with stdout swapped to a `tempfile`. On Unix, the test uses
`std::os::fd::AsFd` to wrap a real non-TTY fd; on test-environment
platforms that don't support fd swapping, the test may inject an
`IsTerminal`-replacement seam (CLI-side pure-function extraction
keeps the test simple).

**Sim/real adapter substitutions**: real reqwest against a bound
localhost test server; real filesystem write.

**Asserts**: same as S-CLI-01 plus stdout file content is a single
JSON object.

#### S-CLI-03 — Piping CLI output to jq returns a single line with the spec digest (T3)

```gherkin
Scenario: Piping CLI output to jq returns a single line with the spec digest
  Given a control plane is running
  When the operator runs `overdrive job submit ./payments.toml | jq -r .spec_digest`
  Then jq's output is a single line of 64 hex characters
  And the CLI exits with status 0
```

**Driving port**: real subprocess invocation: `bash -c "overdrive job
submit ... | jq -r .spec_digest"`.

**Sim/real adapter substitutions**: NONE — real `overdrive` binary,
real `jq`, real shell pipeline.

**Lima VM gate**: macOS via `cargo xtask lima run --`. CI on Linux
without prefix.

**Asserts**: stdout from the shell pipeline matches `^[0-9a-f]{64}$`;
the shell pipeline exit status is 0.

**Note**: this scenario explicitly verifies US-04 AC #2 ("`submit | jq
-r .spec_digest` works without `--detach`"). The `jq` dependency is
acceptable per existing project test conventions; if `jq` is not
guaranteed present, the test substitutes a Rust JSON-extracting
helper but keeps the shell-pipeline shape.

#### S-CLI-04 — Failed terminal event renders structured Error block with reproducer (T1)

```gherkin
Scenario: Failed terminal event renders structured Error block with reproducer
  Given the CLI submit command consumes a stream that ends with `ConvergedFailed`
  And the terminal event carries reason `driver_start_failed` and error text `"stat /no/such: no such file or directory"`
  When the CLI prints its terminal block
  Then the output contains `Error: job '<name>' did not converge to running.`
  And the output contains the literal driver error text
  And the output contains a line starting with `reproducer:` referencing `alloc status --job <name>`
  And the CLI exits with status 1
```

**Driving port**: the CLI's `submit` command function, invoked
in-process. The HTTP backend is a fake that emits a pre-recorded
NDJSON sequence ending in `ConvergedFailed`.

**Sim/real adapter substitutions**: in-process axum returning a fixed
NDJSON stream; CLI's reqwest client + NDJSON consumer; real
`println!`-equivalent capturing into a test buffer.

**Asserts**: stdout contains the four substrings; exit-code accumulator
is 1.

#### S-CLI-05 — Pre-Accepted HTTP error exits 2 (T1)

```gherkin
Scenario: Pre-Accepted HTTP error exits 2
  Given the CLI submit command sends a request
  And the control plane returns 400 Bad Request with an ErrorBody JSON body
  When the CLI processes the response
  Then the CLI prints the ErrorBody message to the operator
  And the CLI exits with status 2
```

**Driving port**: in-process CLI invocation against a fake HTTP
backend returning a pre-recorded 4xx response.

**Asserts**: CLI exit code is 2 (matching ADR-0032 §9 mapping);
stdout/stderr carries the `ErrorBody.message` substring.

**Variant**: parametrise across 400 / 404 / 409 / 500 / transport-
error (no HTTP response); every case must produce exit code 2.

#### S-CLI-06 — TTY without --detach selects NDJSON streaming (T1)

```gherkin
Scenario: TTY without --detach selects NDJSON streaming
  Given the CLI's stdout is a TTY
  And the --detach flag is not present
  When the CLI submit command runs
  Then the request's Accept header is `application/x-ndjson`
  And the CLI engages the NDJSON line-delimited consumer
```

**Driving port**: in-process CLI invocation with a faked-TTY stdout
seam (the CLI exposes an `IsTerminal`-injection point for testability;
production wires it to `std::io::IsTerminal`).

**Asserts**: the test backend captures the `application/x-ndjson`
Accept header.

---

### 3.4 `alloc status` snapshot scenarios (T1)

Exercise the snapshot endpoint's hydration logic and the CLI render
contract.

#### S-AS-01 — AllocStatusResponse carries the six new fields in the Running case

```gherkin
Scenario: AllocStatusResponse carries the six new fields in the Running case
  Given an allocation is in the Running state with started_at set
  And the lifecycle reconciler view records 0 restart attempts
  When the alloc_status handler hydrates the snapshot
  Then the response carries job_id, spec_digest, replicas_desired, and replicas_running at the top level
  And the response carries restart_budget at the top level
  And the row carries state, resources, started_at, last_transition, and (None) error
  And the response serialises six populated actionable fields beyond the original alloc_id/job_id/node_id/state set
```

**Driving port**: `axum::Router::oneshot` on `GET /v1/allocs?job=<id>`.

**Sim/real adapter substitutions**: sim observation store seeded with
a Running row carrying `reason: Some(TransitionReason::Started)`,
`detail: Some("driver started (pid 12345)")`; sim view-cache returning
a `JobLifecycleView` with `restart_counts.values().sum() == 0`.

**Asserts**: count populated fields; structural equality against a
fixture snapshot of the journey TUI mockup.

**KPI binding**: KPI-03 (snapshot field count ≥ 6).

#### S-AS-02 — TransitionRecord and SubmitEvent::LifecycleTransition share the same TransitionReason enum

```gherkin
Scenario: TransitionRecord and SubmitEvent::LifecycleTransition share the same TransitionReason enum
  Given the type AllocStatusRowBody.last_transition is Option<TransitionRecord>
  And TransitionRecord.reason has type TransitionReason
  And SubmitEvent::LifecycleTransition.reason has type TransitionReason
  When a unit test asserts the Rust types are identical
  Then the test compiles
  And the type-equivalence assertion succeeds
```

**Driving port**: a `#[test]` that uses a type-equality witness
(`fn _check<T: ?Sized>(_: T, _: T) where ...`) or `static_assertions::assert_type_eq_all!`.

**Asserts**: compile-time type equality. This is the Tier-1 enforcement
of ADR-0032/0033's [C6] single-source-of-truth.

**Mandate binding**: KPI-04 structural property — the same enum, by
construction.

#### S-AS-03 — utoipa schema declares both content types on POST /v1/jobs and the new fields on AllocStatusResponse

```gherkin
Scenario: utoipa schema declares both content types and new snapshot fields
  Given the project's OpenAPI document is regenerated from utoipa
  When the schema is inspected
  Then `POST /v1/jobs` declares both `application/json` (SubmitJobResponse) and `application/x-ndjson` (SubmitEvent) on the 200 response
  And `AllocStatusResponse` declares fields job_id, spec_digest, replicas_desired, replicas_running, rows, restart_budget
  And `AllocStatusRowBody` declares fields resources, started_at, exit_code, last_transition, error in addition to the existing alloc_id/job_id/node_id/state/reason
```

**Driving port**: the existing `cargo xtask openapi-check` gate plus
a new `#[test]` that diffs the live `utoipa` derivation against the
expected shape.

**Asserts**: openapi-check passes on the regenerated YAML; the new
schema entries are present.

**Mandate binding**: ADR-0032 §11; ADR-0009 (schema gate).

#### S-AS-04 — CLI renders the Running TUI mockup

```gherkin
Scenario: CLI renders the Running TUI mockup
  Given a typed AllocStatusResponse describing a Running allocation with last_transition Started
  When the CLI alloc-status renderer runs against the response
  Then stdout matches the journey YAML's Running TUI mockup
  And the output includes a `Restart budget: 0 / 5 used` line
  And the output includes `Last transition: ... Pending → Running reason: driver started (pid 12345) source: driver(process)`
```

**Driving port**: pure rendering function in the CLI crate, invoked
with a typed `AllocStatusResponse` fixture.

**Sim/real adapter substitutions**: NONE — pure renderer.

**Asserts**: stdout byte-equals (or within trivial whitespace
tolerance) the mockup from `discuss/journey-submit-streams-default.yaml`
step 4 / ADR-0033 §4.

**Mandate binding**: US-05 AC #2; KPI-03.

#### S-AS-05 — CLI renders the Failed TUI mockup with the verbatim driver error and (backoff exhausted)

```gherkin
Scenario: CLI renders the Failed TUI mockup with verbatim driver error
  Given a typed AllocStatusResponse describing a Failed allocation
  And the row's error field carries `"stat /usr/local/bin/payments: no such file or directory"`
  And restart_budget.exhausted is true with used=5 and max=5
  When the CLI alloc-status renderer runs against the response
  Then stdout includes the literal verbatim driver error string
  And stdout includes `Restart budget: 5 / 5 used (backoff exhausted)`
  And stdout includes `Last transition: ... Pending → Failed reason: driver start failed source: driver(exec)`
```

**Driving port**: CLI rendering function with a Failed-case fixture.

**Asserts**: stdout structural match against ADR-0033 §4 Failed mockup;
the literal substring assertion catches drift.

**KPI binding**: KPI-02 (Failed-case render carries verbatim driver
error).

#### S-AS-06 — CLI renders Pending-no-capacity with the explicit reason, not silent zero

```gherkin
Scenario: CLI renders Pending-no-capacity with explicit reason
  Given a typed AllocStatusResponse describing a single Pending allocation
  And the row's reason is "no capacity"
  And the row's error is "requested 10000mCPU/10 GiB / free 4000mCPU/3.2 GiB"
  When the CLI alloc-status renderer runs
  Then stdout includes the row in the allocations table
  And stdout includes `reason: no capacity`
  And stdout includes the requested-vs-free text on its own line
  And stdout does NOT render `Allocations: 0` or any silent-zero shape
```

**Driving port**: CLI rendering function with a Pending-case fixture.

**Asserts**: structural match against ADR-0033 §4 Pending-no-capacity
mockup; explicit substring-NOT assertion against `Allocations: 0`.

**Mandate binding**: US-05 AC #3 (empty-state honesty).

#### S-AS-07 — alloc_status handler projects AllocStatusRow.reason to AllocStatusRowBody.last_transition.reason byte-identically

```gherkin
Scenario: alloc_status handler projects row reason to snapshot last_transition reason byte-identically
  Given the ObservationStore holds an AllocStatusRow with reason Some(<R>) and detail Some(<D>)
  When the alloc_status handler hydrates the snapshot
  Then the response's last_transition.reason for that row equals <R>
  And the response's per-row error field equals <D>
```

**Driving port**: `axum::Router::oneshot` on `GET /v1/allocs?job=<id>`.

**Sim/real adapter substitutions**: sim observation store seeded with
the row.

**Property-test shape**: parametrise over all 8 `TransitionReason`
variants × representative `Option<String>` detail values; assert the
projection is identity in every case.

**Mandate binding**: US-06 AC #1; KPI-04.

#### S-AS-08 — restart_budget.exhausted is derived from used >= max consistently

```gherkin
Scenario: restart_budget.exhausted is derived consistently from used and max
  Given a JobLifecycleView with restart_counts summing to N
  And the constant RESTART_BUDGET_MAX = 5
  When the alloc_status handler builds the RestartBudget
  Then exhausted is true iff N >= 5
  And used equals N
  And max equals 5
```

**Driving port**: pure function under test.

**Property-test shape**: parametrise N ∈ [0, 16]; assert the boolean
relationship in every case.

#### S-AS-09 — alloc_status returns 404 when the job does not exist

```gherkin
Scenario: alloc_status returns 404 when the job does not exist
  Given the IntentStore has no job with id "ghost-v0"
  When a request arrives at `GET /v1/allocs?job=ghost-v0`
  Then the response status is 404
  And the body is an ErrorBody with error "not_found" and message "jobs/ghost-v0"
```

**Driving port**: `axum::Router::oneshot`.

**Mandate binding**: ADR-0033 §3 third empty-shape case; ADR-0015
NotFound mapping; ADR-0032 §9 CLI exit-code 2 path.

---

## 4. Tier 1 / Tier 3 split rationale

Per `.claude/rules/testing.md` § "Adding a new test — which tier?":
the question is "what bug class is being defended?" The split below
follows that mechanically.

**Goes to Tier 1 (DST in-process)**: every property-shape, every
serde / rkyv round-trip, every type-equivalence assertion, every
`TransitionReason` enum-projection invariant, every CLI rendering
test, every concurrency-or-timing-or-ordering property of the
streaming handler that a `SimClock` or `SimDriver` can model
faithfully. Most scenarios.

**Goes to Tier 3 (real-kernel integration)**: scenarios whose
load-bearing property is "real syscall propagates correctly through
real subprocess into real exit code." Concretely: the broken-binary
regression target (`S-WS-02`); the happy-path end-to-end
(`S-WS-01`); the jq-pipeline (`S-CLI-03`). Three scenarios; the
minimum that satisfies driving-adapter verification.

**Tier 2 / Tier 4**: not applicable — no eBPF, no perf gate.

This split is the same shape `phase-1-first-workload` adopted: one
walking-skeleton-class scenario per real-driver behaviour, in T3; the
rest in T1.

---

## 5. Property-shape scenarios summary

These scenarios use proptest generators rather than hand-picked
example values. Listed as a separate inventory so the crafter can
scaffold the generators alongside the production types.

| Scenario | Generator domain | Cases |
|---|---|---|
| S-CP-02 | IntentStore commit latency ∈ [0 ms, 100 ms] | 1024 |
| S-CP-04 | sequence of N transitions, N ∈ [1, 32] | 1024 |
| S-CP-07 | every TransitionReason variant | 8 (exhaustive) |
| S-CP-09 | every TransitionReason × Option<String> × every AllocState | 1024 |
| S-CLI-05 | every HTTP status from {400, 404, 409, 500, transport_err} | 5 (exhaustive) |
| S-AS-07 | every TransitionReason × Option<String> | 1024 |
| S-AS-08 | restart count N ∈ [0, 16] | 1024 |

All `@property`-tagged scenarios. The crafter implements them as
proptest functions per the `.claude/rules/testing.md` § "Property-
based testing" rules; default `PROPTEST_CASES = 1024`; seeds printed
on failure.

---

## 6. Out of scope

- `alloc status --follow` / `--watch` flag — DESIGN [C4] / [D4].
- Multi-replica progress aggregation — DESIGN §11; Phase 1 is
  `replicas == 1`.
- TUI mode (ratatui) — slice-02 OUT scope.
- A second streaming endpoint (e.g. `GET /v1/allocs/stream`) —
  DESIGN §11.
- Operator-controlled `--timeout` flag — DESIGN §11.
- Tier 2 (BPF unit) — no eBPF programs touched.
- Tier 4 (verifier / perf) — no eBPF programs touched.

---

## 7. References

- `discuss/wave-decisions.md`, `discuss/user-stories.md`,
  `discuss/outcome-kpis.md`, `discuss/journey-submit-streams-default.yaml`
- `design/wave-decisions.md`, `design/architecture.md`,
  `design/c4-component.md`, `design/reuse-analysis.md`,
  `design/review.yaml`
- `docs/product/architecture/adr-0032-ndjson-streaming-submit.md`
- `docs/product/architecture/adr-0033-alloc-status-snapshot-enrichment.md`
- ADR-0008, ADR-0009, ADR-0011, ADR-0013, ADR-0014, ADR-0015,
  ADR-0021, ADR-0023, ADR-0027, ADR-0028, ADR-0029, ADR-0030
- `.claude/rules/testing.md`, `.claude/rules/development.md`
