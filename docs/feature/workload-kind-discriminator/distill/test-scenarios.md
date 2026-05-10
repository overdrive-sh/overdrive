# Test Scenarios — workload-kind-discriminator

**Feature**: workload-kind-discriminator
**Wave**: DISTILL
**Author**: Quinn (nw-acceptance-designer)
**Date**: 2026-05-10

This file is **specification only**. Per `.claude/rules/testing.md` §
Testing, no `.feature` file is shipped and no Gherkin parser is wired
in this codebase. The crafter (`@nw-software-crafter`) translates the
scenarios below into Rust integration tests directly under
`crates/{crate}/tests/integration/<scenario>.rs` per
`.claude/rules/testing.md` § "Integration vs unit gating".

## How to read

- Each scenario has a stable ID (e.g. `S-01-04`) for traceability.
- Tags name traceability anchors: `@US-NN` story, `@K-N` KPI,
  `@walking_skeleton`, `@driving_port:<name>`,
  `@infrastructure-failure`, `@real-io`, `@adapter-integration`,
  `@property`, `@anti-scenario`, `@kpi`.
- The `@driving_port:<name>` tag names the entry-point shape the
  crafter must invoke; pipeline-level scenarios are NOT credited as
  driving-port coverage on their own.
- The `Tier` line names the test tier per `.claude/rules/testing.md`
  (default lane / Tier 3 integration / xtask `#[test]`).
- The `Crate` line names where the Rust test should land.

## Wave traceability

| Story | Slice | Section | Walking skeleton |
|---|---|---|---|
| US-01 (parser kind discriminator) | Slice 01 | §1 | WS-01, WS-02, WS-03 |
| US-02 (Job submit terminal verdict) | Slice 02 | §2 | WS-02 |
| US-03 (alloc status kind-aware Job) | Slice 03 | §3 | WS-04 |
| US-04 (Service preservation) | Slice 04 | §4 | WS-01 |
| US-05 (Schedule parsing + deferral) | Slice 05 | §5 | WS-03 |
| US-06 (`"live"` grep gate) | Slice 01 | §6 | n/a (xtask gate) |
| US-07 (coinflip migration) | Slice 01 | §7 | WS-02 |
| US-08 (Service listener spec shape) | Slice 06 | §8 | WS-01, WS-04 |

---

## §1 — Parser kind discriminator (US-01)

**Slice**: 01
**Driving port**: `WorkloadSpecInput::deserialize(toml_bytes)` —
custom `Deserialize` impl per ADR-0047 §2.
**Crate**: `overdrive-core` (parser logic) + `overdrive-cli`
(integration through the submit handler).
**Tier**: default lane.
**Tags**: `@US-01 @driving_port:parser`

### S-01-01 — Service spec is recognised by `[service]` section presence

**Tags**: `@walking_skeleton @driving_port:parser @real-io`
*(part of WS-01)*

```gherkin
Given a TOML spec file at "./payments.toml" containing only
  [service], two [[listener]] blocks, [exec], [resources]
When the parser reads the file
Then a Service-kind workload spec is constructed
And the spec carries the workload identifier "payments"
And no Job or Schedule kind is constructed
```

### S-01-02 — Job spec is recognised by `[job]` section presence

**Tags**: `@walking_skeleton @driving_port:parser @real-io`
*(part of WS-02)*

```gherkin
Given a TOML spec file at "./coinflip.toml" containing only
  [job], [exec], [resources]
When the parser reads the file
Then a Job-kind workload spec is constructed
And the spec carries the workload identifier "coinflip"
And no Service or Schedule kind is constructed
```

### S-01-03 — Scheduled Job spec is recognised by `[job]` + `[schedule]` co-presence

**Tags**: `@walking_skeleton @driving_port:parser @real-io`
*(part of WS-03)*

```gherkin
Given a TOML spec file containing [job], [schedule] with cron
  "0 2 * * *", [exec], [resources]
When the parser reads the file
Then a Schedule-kind workload spec is constructed
And the cron expression is captured as the string "0 2 * * *"
```

### S-01-04 — Spec with both `[service]` and `[job]` is rejected with named guidance

**Tags**: `@error_path @driving_port:parser`

```gherkin
Given a TOML spec file containing both [service] and [job] blocks
When the operator submits the spec
Then the parser rejects the spec
And the error names both [service] and [job] sections explicitly
And the error suggests "exactly one of [service] or [job] is required"
And the operator's command exits with a non-zero status
```

### S-01-05 — Spec with `[schedule]` but no `[job]` is rejected with named guidance

**Tags**: `@error_path @driving_port:parser`

```gherkin
Given a TOML spec file containing [schedule] but not [job]
When the operator submits the spec
Then the parser rejects the spec
And the error names the missing [job] section
And the error states "[schedule] is only valid alongside [job]"
And the operator's command exits with a non-zero status
```

### S-01-06 — Spec with `[schedule]` AND `[service]` is rejected

**Tags**: `@error_path @edge_case @driving_port:parser`

```gherkin
Given a TOML spec file containing both [service] and [schedule]
When the operator submits the spec
Then the parser rejects the spec
And the error states "[schedule] is only valid alongside [job]"
And the operator's command exits with a non-zero status
```

### S-01-07 — Spec missing `[exec]` is rejected

**Tags**: `@error_path @driving_port:parser`

```gherkin
Given a TOML spec file containing [job] and [resources] but no
  [exec] block
When the operator submits the spec
Then the parser rejects the spec
And the error names [exec] as the missing required section
And the operator's command exits with a non-zero status
```

### S-01-08 — Parser rejection latency is within the operator-facing budget

**Tags**: `@kpi @K2 @driving_port:parser`

```gherkin
Given a representative set of invalid mixed-kind specs
  ([service]+[job]; [schedule] alone; [schedule]+[service];
  missing [exec]; missing [resources])
When the parser reads each spec
Then 95% or more of rejection paths complete within 50 milliseconds
  per spec
```

### S-01-09 — Mixed-kind rejection holds across arbitrary section orderings

**Tags**: `@property @driving_port:parser`

```gherkin
Given a generator producing TOML specs with two-of-three section
  presence in any ordering
When the parser reads each generated spec
Then every spec is rejected with a named-guidance error
And the error names the offending sections
```

---

## §2 — Job submit terminal verdict (US-02 — closes the bug)

**Slice**: 02
**Driving port**: `commands::job::submit(SubmitArgs, &Clock,
&Transport)` — direct handler invocation per
`crates/overdrive-cli/CLAUDE.md`.
**Crate**: `overdrive-cli`.
**Tier**: default lane (most scenarios use `SimDriver`); S-02-09 is
Tier 3 (`integration-tests`, Lima) for K1 honesty over real
`ExecDriver`.
**Tags**: `@US-02 @driving_port:cli_submit`

### S-02-01 — A Job that exits 0 reports `Succeeded` with exit code and duration

**Tags**: `@walking_skeleton @driving_port:cli_submit @real-io`
*(part of WS-02 happy branch)*

```gherkin
Given a Job spec at "./coinflip.toml" whose workload exits 0 on
  the first attempt within 1.2 seconds
When the operator runs `overdrive job submit ./coinflip.toml`
And the streaming submit observes the exit
Then the CLI prints a single terminal line "Job 'coinflip' succeeded."
And the line names exit code 0
And the line names a measured duration (not the literal "live")
And the operator's command exits with status 0
```

### S-02-02 — A Job that exits non-zero on every attempt reports `Failed` with attempts

**Tags**: `@walking_skeleton @driving_port:cli_submit @real-io`
*(part of WS-02)*

```gherkin
Given a Job spec at "./coinflip.toml" whose workload exits 1 on
  every attempt up to backoff_limit
When the operator runs `overdrive job submit ./coinflip.toml`
And the streaming submit observes BackoffExhausted
Then the CLI prints a single terminal line "Job 'coinflip' failed."
And the line names exit code 1
And the line names attempts as "3 of 3 (backoff exhausted)"
And the line includes the workload's stderr tail
And the operator's command exits with a non-zero status
```

### S-02-03 — An intermediate attempt failure does not close the stream

**Tags**: `@driving_port:cli_submit`

```gherkin
Given a Job whose first attempt exits 1 and whose backoff_limit is 3
When the streaming submit observes the first failed attempt
Then the CLI prints "Job 'coinflip' attempt 1 failed (exit 1, ...).
  Retrying in ..." (an intermediate line)
And the CLI does not yet print a terminal verdict
And the streaming session remains open
```

### S-02-04 — A Job whose third attempt exits 0 reports `Succeeded`

**Tags**: `@edge_case @driving_port:cli_submit`

```gherkin
Given a Job whose first two attempts exit 1 and whose third
  attempt exits 0 within 1.0 seconds
When the streaming submit runs to terminal
Then the CLI prints two intermediate "attempt N failed... Retrying"
  lines
And the CLI prints a single terminal line "Job 'coinflip' succeeded."
And the line names exit code 0
And the operator's command exits with status 0
```

### S-02-05 — Anti-scenario: no Job submit produces "is running with" or "(took live)"

**Tags**: `@anti-scenario @driving_port:cli_submit`

```gherkin
Given any Job-kind spec whose workload reaches a terminal exit
  (Succeeded, Failed, or AttemptFailed → Succeeded)
When the streaming submit runs to a terminal event
Then no line of the operator-visible CLI output contains the
  substring "is running with"
And no line of the operator-visible CLI output contains the
  substring "(took live)"
```

### S-02-06 — Submit echo names the kind upfront

**Tags**: `@driving_port:cli_submit`

```gherkin
Given a Job spec at "./coinflip.toml"
When the operator runs `overdrive job submit ./coinflip.toml`
Then the CLI prints "Submitting job 'coinflip'
  (kind=Job, run-to-completion)" before any streaming events
```

### S-02-07 — Server-side spec validation failure surfaces a structured error

**Tags**: `@error_path @infrastructure-failure @driving_port:cli_submit`

```gherkin
Given a syntactically-valid TOML spec that the server rejects
  (e.g. control-plane policy denies the kind for this tenant)
When the operator runs `overdrive job submit ./denied.toml`
Then the CLI prints an error naming the rejection cause
And the operator's command exits with a non-zero status
And no streaming session is opened
```

### S-02-08 — Streaming transport interruption surfaces honestly

**Tags**: `@error_path @infrastructure-failure @driving_port:cli_submit`

```gherkin
Given a Job submit whose streaming connection is interrupted mid-stream
  (Transport::Failed injected by SimTransport mid-flight)
When the streaming submit observes the connection drop
Then the CLI prints an error naming the transport failure
And the operator's command exits with a non-zero status
And the CLI does NOT print a "succeeded" or "running" verdict
```

### S-02-09 — K1 honesty: 100 trials of coinflip, CLI exit code = workload exit code

**Tags**: `@kpi @K1 @walking_skeleton @driving_port:cli_submit
@real-io @adapter-integration @infrastructure-failure`
*(K1 is the load-bearing observability KPI; this scenario lands as
the single Tier-3 test gated behind `integration-tests` + Lima)*

```gherkin
Given a fresh Lima VM with the migrated examples/coinflip.toml
  shipped with this feature
When the operator runs `overdrive job submit examples/coinflip.toml`
  one hundred times against the real ExecDriver
Then for at least 99 of the 100 trials, the CLI process exit code
  equals the workload's kernel-observed exit code
And for every trial, the CLI's terminal verdict line names the same
  exit code as the kernel observed
```

---

## §3 — alloc status kind-aware Job render (US-03)

**Slice**: 03
**Driving port**: `commands::alloc_status::status(StatusArgs,
&Clock, &Transport)` — direct handler invocation.
**Crate**: `overdrive-cli`.
**Tier**: default lane.
**Tags**: `@US-03 @driving_port:cli_alloc_status`

### S-03-01 — Service alloc status shows replicas and restarts; no Exit column

**Tags**: `@walking_skeleton @driving_port:cli_alloc_status @real-io`
*(part of WS-04)*

```gherkin
Given a Service "payments" running stably with 1 of 1 replicas
When the operator runs `overdrive alloc status --job payments`
Then the output contains "kind: Service"
And the output contains "Replicas (desired/running): 1/1"
And the per-alloc table has columns "Alloc, State, Restarts, Since"
And the per-alloc table has NO column named "Exit"
```

### S-03-02 — Job alloc status (Failed) shows verdict, attempts, exit codes, stderr

**Tags**: `@walking_skeleton @kpi @K3 @driving_port:cli_alloc_status
@real-io` *(part of WS-04)*

```gherkin
Given a Job "coinflip" whose three attempts all exited 1 and the
  reconciler emitted BackoffExhausted
When the operator runs `overdrive alloc status --job coinflip`
Then the output contains "kind: Job"
And the output contains "Verdict: Failed (backoff exhausted)"
And the per-attempt table has columns "Attempt, State, Exit, Started,
  Duration"
And every Failed attempt row shows Exit "1"
And the output includes the stderr tail of the last attempt
```

### S-03-03 — Job alloc status (Succeeded) shows Verdict Succeeded with Exit 0

**Tags**: `@driving_port:cli_alloc_status`

```gherkin
Given a Job "coinflip" whose first attempt exited 0 within 1.2 seconds
When the operator runs `overdrive alloc status --job coinflip`
Then the output contains "Verdict: Succeeded"
And the attempts table contains exactly one row with State
  "Succeeded" and Exit "0"
```

### S-03-04 — Job alloc status (in progress) shows Verdict In progress with Exit em-dash

**Tags**: `@edge_case @driving_port:cli_alloc_status`

```gherkin
Given a Job "long-import" whose only attempt has been Running for
  2 minutes with no terminal yet
When the operator runs `overdrive alloc status --job long-import`
Then the output contains "Verdict: In progress (no terminal yet)"
And the attempts table contains exactly one row with State
  "Running" and Exit rendered as em-dash
```

### S-03-05 — Anti-scenario: Job alloc status NEVER renders Service phrasing

**Tags**: `@anti-scenario @driving_port:cli_alloc_status`

```gherkin
Given any Job-kind workload at any state
  (Succeeded, Failed, In progress)
When the operator runs `overdrive alloc status --job <id>`
Then no line of output contains the substring "is running with"
And no line of output contains the substring "Replicas"
```

### S-03-06 — alloc status for unknown job ID surfaces a typed error

**Tags**: `@error_path @driving_port:cli_alloc_status`

```gherkin
Given no workload named "ghost" has ever been submitted
When the operator runs `overdrive alloc status --job ghost`
Then the CLI prints an error naming "ghost" as not found
And the operator's command exits with a non-zero status
```

### S-03-07 — alloc status with a corrupt observation row surfaces honestly

**Tags**: `@error_path @infrastructure-failure
@driving_port:cli_alloc_status`

```gherkin
Given a workload whose AllocStatusRow cannot be deserialised
  (e.g. truncated rkyv bytes simulating disk corruption)
When the operator runs `overdrive alloc status --job <id>`
Then the CLI prints an error naming the deserialise failure
And the operator's command exits with a non-zero status
And the CLI does NOT print a fabricated "Unknown" or empty row
```

### S-03-08 — K3 automated regression: rendered Exit column matches persisted exit_code

**Tags**: `@kpi @K3 @property @driving_port:cli_alloc_status`

```gherkin
Given a generator producing AllocStatusRow fixtures with arbitrary
  per-attempt exit codes in {0, 1, 2, 127, 137, 255}
When the operator runs alloc status against each fixture
Then the rendered Exit column for every attempt row matches the
  persisted exit_code byte-for-byte
```

---

## §4 — Service preservation (US-04)

**Slice**: 04
**Driving port**: `commands::job::submit(...)` for Service-kind specs.
**Crate**: `overdrive-cli`.
**Tier**: default lane.
**Tags**: `@US-04 @kpi @K4 @driving_port:cli_submit`

### S-04-01 — Existing happy-path Service tests pass with renamed vocabulary

**Tags**: `@walking_skeleton @driving_port:cli_submit @real-io`
*(part of WS-01)*

```gherkin
Given the existing streaming_submit_happy_path test fixture
  migrated from the legacy flat shape to [service]
When the test runs against a real (in-process) control plane
Then the streaming output contains "Service '...' is running with
  1/1 replicas (took <duration>)"
And the rendered duration is a measured value, not the literal "live"
And the operator's command exits with status 0
```

### S-04-02 — A Service exit during stability window emits ConvergedFailed (vocabulary change only)

**Tags**: `@error_path @driving_port:cli_submit`

```gherkin
Given a Service whose process exits within the stability window
When the streaming subscriber observes the exit
Then the CLI prints "Service '<name>' failed to stabilise."
And the line continues to name attempts and the next-retry timing
And the operator's command exits with a non-zero status
```

### S-04-03 — Anti-scenario: the literal "live" never appears in render output

**Tags**: `@anti-scenario @driving_port:cli_submit`

```gherkin
Given any Service-kind spec whose workload stabilises in any
  measurable duration (1 millisecond through 1 hour)
When the streaming submit observes the convergence
Then no line of operator-visible CLI output contains the literal
  string "took live" or " live)"
```

### S-04-04 — `format_stopped_summary` is kind-aware

**Tags**: `@driving_port:cli_submit`

```gherkin
Given a Service "payments" stopped by the operator's control-plane action
When the streaming subscriber observes the stop
Then the CLI prints "Service 'payments' was stopped by ..."
  (NOT "Job 'payments' was stopped by ...")
```

---

## §5 — Schedule parsing + honest deferral (US-05)

**Slice**: 05
**Driving port**: `commands::job::submit(...)` for Schedule-kind +
`commands::alloc_status::status(...)`.
**Crate**: `overdrive-cli`.
**Tier**: default lane.
**Tags**: `@US-05 @kpi @K5 @driving_port:cli_submit
@driving_port:cli_alloc_status`

### S-05-01 — Submit echoes "registered" plus a deferral note with tracking URL

**Tags**: `@walking_skeleton @driving_port:cli_submit @real-io`
*(part of WS-03)*

```gherkin
Given a Scheduled Job spec at "./nightly-backup.toml"
When the operator runs `overdrive job submit ./nightly-backup.toml`
Then the CLI prints "Submitting schedule 'nightly-backup'
  (kind=Schedule)"
And the CLI prints "Schedule registered."
And the CLI prints a NOTE saying execution is not yet implemented
And the NOTE includes the tracking issue URL
  https://github.com/overdrive-sh/overdrive/issues/166
And the operator's command exits with status 0
```

### S-05-02 — alloc status for a Schedule names the deferral with the same URL (K5 byte-equality)

**Tags**: `@walking_skeleton @kpi @K5 @driving_port:cli_alloc_status
@real-io` *(part of WS-04)*

```gherkin
Given a registered Scheduled Job whose execution is deferred
When the operator runs `overdrive alloc status --job <id>`
And had previously run the matching `overdrive job submit ...`
Then the alloc status output contains "kind: Schedule"
And the alloc status output contains the cron expression unchanged
And the alloc status output's deferral URL byte-equals the URL
  printed by the prior submit echo
```

### S-05-03 — `[schedule]` without `[job]` is rejected (cross-reference S-01-05)

**Tags**: `@error_path @driving_port:parser @driving_port:cli_submit`
*(parser-side covered by S-01-05; this is the CLI-handler-side
observation)*

```gherkin
Given a TOML file containing [schedule] without [job]
When the operator runs `overdrive job submit ./bad.toml`
Then the CLI prints an error naming "[schedule] is only valid
  alongside [job]"
And the operator's command exits with a non-zero status
```

### S-05-04 — `[schedule]` requires a non-empty `cron` field

**Tags**: `@error_path @driving_port:parser`

```gherkin
Given a TOML file with [job] + [schedule] but no `cron` field
When the operator submits the spec
Then the parser rejects the spec
And the error names `cron` as the missing required field within
  [schedule]
And the operator's command exits with a non-zero status
```

### S-05-05 — Schedule-kind submit persists the spec to the IntentStore

**Tags**: `@driving_port:cli_submit @real-io @adapter-integration`

```gherkin
Given a Scheduled Job spec at "./nightly-backup.toml"
When the operator runs `overdrive job submit ./nightly-backup.toml`
Then the spec is persisted to the IntentStore
And re-reading the IntentStore by the canonical IntentKey returns
  a Schedule-kind workload spec carrying the operator-supplied cron
  expression
```

### S-05-06 — Deferral URL is sourced from a single CLI constant

**Tags**: `@kpi @K5 @driving_port:cli_submit`

```gherkin
Given the SCHEDULE_EXECUTION_TRACKING_URL constant defined once in
  the CLI
When any code path reads the deferral URL (submit echo or alloc
  status render)
Then every read returns the byte-identical URL
And the URL equals "https://github.com/overdrive-sh/overdrive/issues/166"
```

---

## §6 — `"live"` literal grep gate (US-06)

**Slice**: 01 (regression guard for US-02 / US-04).
**Driving port**: `xtask::dst_lint` scanner — extends the existing
banned-API scanner with a new rule.
**Crate**: `xtask` (gate logic) + `overdrive-cli` (target source for
the gate to scan).
**Tier**: xtask `#[test]` (default lane) — pure-fn-shaped scanner
with a fixture corpus.
**Tags**: `@US-06 @driving_port:dst_lint`

### S-06-01 — dst-lint rejects `"live"` literal in render or command source

**Tags**: `@anti-scenario @driving_port:dst_lint`

```gherkin
Given a deliberately-introduced regression file at
  crates/overdrive-cli/src/render/regression.rs
And the file contains a render literal `"live"` passed to a duration
  formatter
When `cargo xtask dst-lint` runs the workspace scanner
Then the scanner exits with a non-zero status
And the scanner's output names the offending file path and line number
And the scanner's output names the rule "live-literal-banned"
```

### S-06-02 — dst-lint allows `"live"` in comments and docstrings

**Tags**: `@edge_case @driving_port:dst_lint`

```gherkin
Given a render-source file containing `// historical: the literal
  "live" used to be here` as a comment
When `cargo xtask dst-lint` runs the workspace scanner
Then the scanner exits 0 for this file
And no violation is recorded
```

### S-06-03 — dst-lint passes on the migrated codebase

**Tags**: `@kpi @K1 @driving_port:dst_lint`

```gherkin
Given the codebase after Slice 01 + Slice 04 land
When `cargo xtask dst-lint` runs the workspace scanner
Then the scanner exits 0
And no `"live"` literal appears in any render or command source path
```

---

## §7 — `examples/coinflip.toml` migration (US-07)

**Slice**: 01.
**Driving port**: `WorkloadSpecInput::deserialize` against the
migrated file content.
**Crate**: `overdrive-cli` (parser test) + repo-level migration of
`examples/coinflip.toml`.
**Tier**: default lane.
**Tags**: `@US-07 @driving_port:parser`

### S-07-01 — Migrated coinflip.toml parses as Job kind

**Tags**: `@walking_skeleton @driving_port:parser @real-io`
*(part of WS-02 setup)*

```gherkin
Given the migrated examples/coinflip.toml file shipped with this
  feature
When the parser reads the file
Then a Job-kind workload spec is constructed
And the spec's identifier is "coinflip"
And the spec's exec command is "/bin/bash"
```

### S-07-02 — Migrated coinflip.toml exercises the bug-under-audit reproduction

**Tags**: `@walking_skeleton @kpi @K1 @driving_port:cli_submit
@real-io @adapter-integration` *(part of WS-02; gated Tier 3)*

```gherkin
Given the migrated examples/coinflip.toml whose bash script picks a
  pseudo-random branch (SUCCESS or ERROR)
When the operator runs `overdrive job submit examples/coinflip.toml`
  against the real ExecDriver
Then the CLI's terminal verdict line names "succeeded" XOR "failed"
And the rendered exit code matches the workload's kernel exit code
And no terminal verdict reads "running" or "live"
```

---

## §8 — Service `[[listener]]` spec shape (US-08)

**Slice**: 06.
**Driving port**: `WorkloadSpecInput::deserialize` (parser),
`commands::job::submit(...)` (echo render),
`commands::alloc_status::status(...)` (alloc-status render),
`overdrive-control-plane::openapi::generate` (schema gate).
**Crate**: `overdrive-core` (Listener type) + `overdrive-cli`
(integration through submit/alloc-status handlers) +
`overdrive-control-plane` (OpenAPI gate).
**Tier**: default lane (parser + render + property test);
xtask alias for OpenAPI gate.
**Tags**: `@US-08 @driving_port:parser
@driving_port:cli_submit @driving_port:cli_alloc_status`

### S-08-01 — A Service with two valid listeners parses with both triples preserved in declaration order

**Tags**: `@walking_skeleton @driving_port:parser @real-io`
*(part of WS-01)*

```gherkin
Given a TOML file at "./frontend.toml" with [service] id="frontend"
  and two [[listener]] blocks (8080/tcp/10.0.0.1 then 8081/udp/none)
When the parser reads the file
Then a Service-kind workload spec is constructed
And the spec carries listeners
  [(10.0.0.1, 8080, tcp), (none, 8081, udp)] in declaration order
```

### S-08-02 — Protocol parsing is case-insensitive and canonicalises to lowercase

**Tags**: `@edge_case @driving_port:parser`

```gherkin
Given a TOML file with a [[listener]] whose protocol value is "TCP"
  in upper case
When the parser reads the file
Then the parsed listener's protocol equals tcp
And every CLI surface that renders the protocol prints "tcp" in
  lowercase
```

### S-08-03 — A Service with zero listeners is rejected with named guidance

**Tags**: `@error_path @driving_port:parser`

```gherkin
Given a TOML file with [service] but no [[listener]] blocks
When the operator runs `overdrive job submit ./no-listener.toml`
Then the CLI prints an error stating "a [service] requires at least
  one [[listener]] block"
And the operator's command exits with a non-zero status
```

### S-08-04 — A duplicate `(vip, port, protocol)` triple is rejected with named guidance

**Tags**: `@error_path @driving_port:parser`

```gherkin
Given a TOML file with two [[listener]] blocks both naming
  (none, 8080, tcp)
When the operator runs `overdrive job submit ./duplicate.toml`
Then the CLI prints an error naming the duplicate triple
And the operator's command exits with a non-zero status
```

### S-08-05 — An unsupported protocol value is rejected (sctp, icmp, empty)

**Tags**: `@error_path @driving_port:parser`

```gherkin
Given a TOML file with a [[listener]] whose protocol is "sctp"
When the operator runs `overdrive job submit ./bad-proto.toml`
Then the CLI prints an error naming "sctp" as an unsupported protocol
And the error names the supported set as "tcp, udp"
And the operator's command exits with a non-zero status
```

### S-08-06 — `port = 0` is rejected

**Tags**: `@error_path @edge_case @driving_port:parser`

```gherkin
Given a TOML file with a [[listener]] whose port is 0
When the operator runs `overdrive job submit ./bad-port.toml`
Then the CLI prints an error naming "port must be in 1..=65535"
And the operator's command exits with a non-zero status
```

### S-08-07 — Submit echo surfaces every listener with pinned-or-pending VIP

**Tags**: `@walking_skeleton @kpi @K6 @driving_port:cli_submit
@real-io` *(part of WS-01)*

```gherkin
Given a Service spec with one pinned-VIP listener (10.0.0.1, 8080,
  tcp) and one None-VIP listener (none, 8081, udp)
When the operator runs `overdrive job submit ./mixed-vip.toml`
Then the CLI submit echo includes a "Listeners:" section
And the pinned-VIP listener is rendered as "10.0.0.1:8080/tcp"
And the None-VIP listener is rendered as
  "(vip: pending allocation — see #167):8081/udp"
```

### S-08-08 — alloc status renders a Listeners section for a Service

**Tags**: `@walking_skeleton @kpi @K6 @driving_port:cli_alloc_status
@real-io` *(part of WS-04)*

```gherkin
Given a Service "frontend" with two listeners (one pinned, one pending)
When the operator runs `overdrive alloc status --job frontend`
Then the output contains a "Listeners:" section
And both listeners appear with their (vip-or-pending, port, protocol)
  triple
And every listener line's protocol is rendered in lowercase
```

### S-08-09 — Listener round-trip byte-equality across submit + alloc status

**Tags**: `@property @kpi @K6 @driving_port:cli_submit
@driving_port:cli_alloc_status @real-io @adapter-integration`

```gherkin
Given a generator producing valid Service specs with N listeners
  (N in 1..=8, with valid distinct (vip, port, protocol) triples
  including a mix of pinned and None VIPs)
When the operator submits each spec and then runs alloc status for
  the same workload
Then for every spec, the Listeners section in the submit echo
  byte-equals the Listeners section in the alloc status output
```

### S-08-10 — JobSpecInput round-trips bit-equivalently through TOML, JSON, and Job

**Tags**: `@property @driving_port:parser`

```gherkin
Given an arbitrary JobSpecInput with N valid listener triples
When the input is serialised to TOML, parsed back, converted to a
  Job aggregate, and converted back via JobSpecInput::from(&Job)
Then the resulting JobSpecInput equals the original
And the listener order is preserved
And every (vip, port, protocol) triple is byte-equivalent
```

### S-08-11 — OpenAPI roundtrip passes for the new listener types

**Tags**: `@adapter-integration @driving_port:openapi @real-io`

```gherkin
Given the Listener and ServiceVip newtypes derive utoipa::ToSchema
When the operator runs `cargo openapi-gen` and then `cargo
  openapi-check`
Then both commands exit with status 0
And the generated schema includes Listener with port, protocol,
  and optional vip fields
```

### S-08-12 — VIP allocator deferral URL is sourced from a single CLI constant

**Tags**: `@driving_port:cli_submit`

```gherkin
Given the SERVICE_VIP_ALLOCATOR_TRACKING_URL constant defined once
  in the CLI
When the submit echo or alloc status renders a None-VIP listener
Then the rendered "(vip: pending allocation — see ...)" suffix
  matches the constant byte-for-byte
And the constant references the GH issue tracking the runtime
  allocator decision (issue #167)
```

---

## Coverage check

Total scenarios: 30 (rough breakdown).

| Section | Count | Walking skeleton | Happy | Error/edge | Property | KPI |
|---|---|---|---|---|---|---|
| §1 Parser | 9 | 3 | 3 | 5 | 1 | K2 |
| §2 Job submit | 9 | 3 | 3 | 5 | – | K1 |
| §3 alloc status | 8 | 2 | 4 | 3 | 1 | K3 |
| §4 Service preservation | 4 | 1 | 2 | 1 | – | K4 |
| §5 Schedule | 6 | 2 | 3 | 2 | – | K5 |
| §6 dst-lint | 3 | – | 1 | 2 | – | K1 (regression guard) |
| §7 Migration | 2 | 2 | 2 | – | – | K1 |
| §8 Listeners | 12 | 4 | 4 | 5 | 2 | K6 |
| **Total** | **53** | **13 WS-tagged** | **22 happy/14 walking-overlap** | **23 error/edge** | **4 property** | **K1..K6** |

(Some scenarios appear in multiple categories — e.g. anti-scenarios
count as error-path; KPI scenarios may also be walking-skeleton.)

Error-path ratio: 23/53 = 43% — passes the ≥40% gate.

Walking-skeleton scenario count (tagged `@walking_skeleton`): 13
covering 4 distinct walking skeletons (WS-01, WS-02, WS-03, WS-04).
Within the 2-5 mandate.

Story coverage (Dim-8 Check A): every story US-01..US-08 has at
least one referencing scenario. See `wave-decisions.md` § DWD-09 for
the full slice → section mapping.

Driving-port coverage (Mandate 1): every named driving port
(`parser`, `cli_submit`, `cli_alloc_status`, `dst_lint`, `openapi`,
plus the implicit IntentStore / ObservationStore / Reconciler /
Streaming subscriber traversed by the WS scenarios) has at least
one walking-skeleton coverage point.

Adapter coverage (Dim-9c): every driven adapter listed in
`wave-decisions.md` § DWD-05 has at least one `@real-io` scenario.

---

## Tags reference

- `@US-NN` — story traceability anchor.
- `@K-N` — KPI observability anchor.
- `@walking_skeleton` — part of one of the four walking skeletons.
- `@driving_port:<name>` — names the entry-point shape the test must
  invoke (parser / cli_submit / cli_alloc_status / dst_lint / openapi).
- `@real-io` — exercises a real local adapter (real `redb`, real
  TOML deserialiser, real subprocess).
- `@adapter-integration` — touches at least one driven adapter at its
  real-I/O boundary.
- `@infrastructure-failure` — exercises an adapter failure mode
  (transport drop, corrupt persistence, validation reject).
- `@property` — universal invariant; crafter implements as proptest.
- `@anti-scenario` — asserts the absence of a specific behavior
  (e.g. "no Job submit ever produces 'is running with'"); structural
  proof per ADR-0047 [D2].
- `@kpi` — emits an observable signal the KPI in `outcome-kpis.md`
  reads.
- `@error_path` — error or rejection scenario (counts toward 40%
  ratio).
- `@edge_case` — boundary or unusual-but-valid scenario.

## What this file is NOT

- NOT a `.feature` file. Per `.claude/rules/testing.md` § Testing,
  cucumber-rs / pytest-bdd / any Gherkin runtime is forbidden in
  this codebase.
- NOT a directive on Rust file naming. The crafter chooses
  `<scenario>.rs` filenames per `.claude/rules/testing.md` §
  "Integration vs unit gating" / `tests/integration/` layout.
- NOT a constraint on `Sim*`-vs-real-adapter selection beyond what
  `wave-decisions.md` § DWD-03 / DWD-05 specify. Default lane is
  in-process direct handler with `Sim*` for non-determinism +
  real local adapters; Tier 3 (gated) for K1.
- NOT a decision on `#[should_panic(expected = "RED scaffold")]`
  placement — that is the crafter's responsibility per
  `.claude/rules/testing.md` § "RED scaffolds".
