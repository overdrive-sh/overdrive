<!-- markdownlint-disable MD024 -->

# User Stories — workload-kind-discriminator

## System Constraints

These cross-cutting constraints apply to every story below:

- **Test runner gating**: every acceptance test invocation must be `cargo xtask lima
  run -- cargo nextest run …` per `.claude/rules/testing.md`. No bare `cargo nextest
  run`. No `cargo test` other than `cargo test --doc`.
- **Type-driven design**: per `.claude/rules/development.md` § "Type-driven design",
  the kind discriminator is a Rust enum at the parser boundary, not a string-typed
  field. Mixing kinds is a compile-time error in downstream code, not a runtime
  validation.
- **No `"live"` literal**: per RCA root cause D, the literal string `"live"` may not
  appear as a hard-coded duration in any render code path after this feature lands. A
  CI grep gate or `dst-lint` rule enforces this.
- **Single-cut migrations**: per `feedback_single_cut_greenfield_migrations.md`, the
  existing `examples/coinflip.toml` is migrated in Slice 01 with no compat shim.
- **CLI/control-plane crate convention**: per `crates/overdrive-cli/CLAUDE.md`,
  integration tests call command handlers directly (no subprocess). All acceptance
  criteria here that test CLI behaviour assume direct-handler invocation.
- **Greenfield Phase 1**: rows persisted before this feature do not need backfill —
  Phase 1 has no production data to preserve.

---

## US-01: Parser kind discriminator

### Problem

Ana, Overdrive platform engineer, has a `coinflip.toml` workload that exits with
status 1 on roughly half its runs. When she submits it through the current Phase 1
CLI, she gets `Job 'coinflip' is running with 1/1 replicas (took live)` regardless of
the workload's actual exit code. She finds it impossible to distinguish a one-shot
script from a long-running service in the spec — every workload is treated as a
service. She wants to write `[job]` for a one-shot and `[service]` for a long-running
process, and have the platform recognise her intent at parse time.

### Who

- Overdrive platform engineer (Ana) | local single-mode dev cluster | wants the spec
  shape to encode lifecycle intent so the platform's behaviour matches.

### Solution

A `WorkloadKind` enum at the spec-parser boundary with three variants
(`Service`/`Job`/`Schedule`), recognised by section presence in TOML. Mixed-kind
specs and missing required sections are rejected with named guidance.

### Domain Examples

#### 1: Happy path — Ana writes a Service spec for `payments`

Ana has a long-running `payments-server` binary. She writes:
```toml
[service]
id = "payments"
replicas = 1
[exec]
command = "/usr/local/bin/payments-server"
args = ["--port", "8080"]
[resources]
cpu_milli = 500
memory_bytes = 268435456
```
The parser produces `WorkloadSpec::Service { id: "payments", replicas: 1, ... }`. No
Job or Schedule fields are constructed.

#### 2: Edge case — Ana writes a Scheduled Job for `nightly-backup`

Ana has a recurring backup. She writes:
```toml
[job]
id = "nightly-backup"
[schedule]
cron = "0 2 * * *"
[exec]
command = "/usr/bin/pg_dump"
args = ["--format=custom", "-d", "payments", "-f", "/backups"]
[resources]
cpu_milli = 200
memory_bytes = 134217728
```
The parser produces `WorkloadSpec::Schedule { job_inner: { id: "nightly-backup", ... },
cron_expr: "0 2 * * *" }`.

#### 3: Error — Ana accidentally writes both `[service]` and `[job]`

Ana copies an old service spec and forgets to remove the `[service]` block when
adding `[job]`. She submits the file. The CLI prints:
```
Error: spec at './ambiguous.toml' contains both [service] and [job] sections
       (lines 1 and 7). Exactly one of [service] or [job] is required.
```
Exit code: non-zero.

### UAT Scenarios (BDD)

#### Scenario: A Service spec is recognised by [service] section presence

```
Given a TOML file at "./payments.toml" containing only [service], [exec], [resources]
When the parser reads the file
Then a Service-kind workload spec is constructed
And neither Job nor Schedule kinds are constructed
```

#### Scenario: A Job spec is recognised by [job] section presence

```
Given a TOML file at "./coinflip.toml" containing only [job], [exec], [resources]
When the parser reads the file
Then a Job-kind workload spec is constructed
```

#### Scenario: A spec with both [service] and [job] is rejected with named guidance

```
Given a TOML file containing both [service] and [job] blocks
When the operator runs `overdrive job submit ./mixed.toml`
Then the CLI prints an error naming both sections as the conflict
And the CLI suggests "exactly one of [service] or [job] is required"
And the CLI exits with a non-zero status code
```

#### Scenario: A spec missing [exec] is rejected

```
Given a TOML file with [job] and [resources] but no [exec] block
When the operator runs `overdrive job submit ./bad.toml`
Then the CLI prints an error naming [exec] as the missing required section
And the CLI exits with a non-zero status code
```

#### Scenario: examples/coinflip.toml is migrated to the [job] shape and parses

```
Given the migrated examples/coinflip.toml file shipped with this feature
When the parser reads the file
Then a Job-kind workload spec is constructed with id "coinflip"
```

### Acceptance Criteria

- [ ] Parser produces `WorkloadKind::{Service, Job, Schedule}` from section presence.
- [ ] Mixed `[service]` + `[job]` rejected with named guidance.
- [ ] `[schedule]` without `[job]` rejected with named guidance.
- [ ] `[schedule]` with `[service]` rejected with named guidance.
- [ ] Missing `[exec]` rejected.
- [ ] `examples/coinflip.toml` is migrated to the `[job]` shape and parses.
- [ ] Spec-validation latency p95 < 50ms for invalid combinations.

### Outcome KPIs

- **Who**: Overdrive platform engineers writing workload specs.
- **Does what**: write specs with explicit kind discrimination.
- **By how much**: 100% of new specs use the new shape; 100% of mixed-kind specs are
  rejected with named guidance (vs. 0% rejection today — the current parser accepts
  whatever and ignores extras).
- **Measured by**: parser unit tests + integration tests + grep gate on
  `examples/`.
- **Baseline**: 0% kind-explicit specs (the field does not exist).

### Technical Notes

- Section-as-discriminator pattern requires a custom `Deserialize` impl or careful
  use of `serde(untagged)` enums.
- Parse error messages must name the offending section, not just "deserialize
  failed". This drives the impl shape.
- Trace: J-OPS-002 (primary).

---

## US-02: Job submit terminates on terminal exit (closes the bug)

### Problem

Ana runs `overdrive job submit examples/coinflip.toml` against an exit-1 workload and
gets `Job 'coinflip' is running with 1/1 replicas (took live)` followed by a process
exit code of 0. Then she sees `ERROR` in the `serve` log and realises the CLI lied to
her. She loses trust in the CLI. She wants the streaming submit to wait for the
workload's terminal exit and report the truth: Succeeded with exit 0, or Failed with
exit non-zero and an attempt count.

### Who

- Overdrive platform engineer (Ana) | submitting one-shot Jobs | wants the CLI's
  verdict to match the kernel's exit code.

### Solution

A `JobSubmitEvent` enum that has NO `ConvergedRunning` variant. The streaming
subscriber for a Job-kind alloc waits for the ExitObserver's terminal observation
row, then emits `Succeeded { exit_code: 0, .. }` or `Failed { exit_code: N, .. }`. The
CLI's render functions for Job kind do not include "is running with N/M replicas".
The CLI process exits with the workload's exit code.

### Domain Examples

#### 1: Happy path — Ana submits a Job that exits 0

Ana submits `examples/coinflip.toml` and the bash workload happens to take the
SUCCESS branch (exit 0, 1.2s).
- CLI streaming output: `Job 'coinflip' succeeded.\n  exit code: 0\n  duration: 1.2s\n  attempts: 1`.
- CLI process exit: 0.

#### 2: Failure — Ana submits the same Job and it exits 1 every attempt

Ana submits the same spec and the bash workload takes the ERROR branch on every
attempt up to `backoff_limit = 3`.
- Intermediate CLI lines: `Job 'coinflip' attempt 1 failed (exit 1, 0.2s). Retrying
  in 0.5s... (attempt 2/3)` (and similar for attempts 2, 3).
- Final CLI streaming output: `Job 'coinflip' failed.\n  exit code: 1\n  duration:
  0.3s (per-attempt)\n  attempts: 3 of 3 (backoff exhausted)\n  stderr (last 5 lines):
  ERROR`.
- CLI process exit: non-zero (1).

#### 3: Mixed — Ana submits and attempts 1 and 2 fail but attempt 3 exits 0

The streaming subscriber emits two `attempt failed` intermediate events, then
`Succeeded` once attempt 3 exits 0.
- CLI process exit: 0.

### UAT Scenarios (BDD)

#### Scenario: A Job that exits 0 reports Succeeded with exit_code and duration

```
Given a Job spec at "./coinflip.toml" whose workload exits 0 on its first attempt within 1.2 seconds
When the streaming submit observes the exit
Then the CLI prints a single terminal line "Job 'coinflip' succeeded."
And the line names exit code 0
And the line names a measured duration (not the literal "live")
And the CLI process exits with status 0
```

#### Scenario: A Job that exits non-zero on every attempt reports Failed with attempts

```
Given a Job spec whose workload exits 1 on every attempt up to backoff_limit
When the streaming submit observes the final BackoffExhausted
Then the CLI prints a single terminal line "Job 'coinflip' failed."
And the line names exit code 1
And the line names attempts as "3 of 3 (backoff exhausted)"
And the line includes the stderr tail
And the CLI process exits with a non-zero status code
```

#### Scenario: An intermediate attempt failure does not close the stream

```
Given a Job whose first attempt exits 1 and whose backoff_limit is 3
When the streaming submit observes the first failed attempt
Then the CLI prints "Job 'coinflip' attempt 1 failed (exit 1, ...). Retrying in ..."
And the stream remains open awaiting the next attempt's outcome
```

#### Scenario: The structural anti-scenario — Job submit cannot render Service phrasing

```
Given any Job spec
When the streaming submit runs to a terminal event
Then no line of CLI output contains the substring "is running with"
And no line of CLI output contains the substring "(took live)"
```

### Acceptance Criteria

- [ ] `JobSubmitEvent` does not include a `ConvergedRunning` variant.
- [ ] Job streaming subscriber waits for ExitObserver terminal row before emitting
      Succeeded/Failed.
- [ ] CLI prints `Job '<id>' succeeded.` for exit 0; CLI exits 0.
- [ ] CLI prints `Job '<id>' failed.` for backoff-exhausted non-zero exit; CLI exits
      non-zero.
- [ ] Intermediate `attempt N failed... Retrying` line printed for each non-final
      failed attempt.
- [ ] Anti-scenario test passes: no Job submit produces "is running with" or
      "(took live)" in any output line.

### Outcome KPIs

- **Who**: Overdrive platform engineers submitting one-shot Jobs.
- **Does what**: receive a CLI verdict that matches the kernel's exit code.
- **By how much**: honesty rate ≥99% over 100 trials of the coinflip workload (both
  branches). Today's baseline: 0% — the CLI says "running" 100% of the time
  regardless of exit.
- **Measured by**: integration test that runs the coinflip workload 100 times and
  asserts the CLI's exit code matches the workload's exit code.
- **Baseline**: 0% (every Job submit reports "running with 1/1 replicas (took live)"
  regardless of actual outcome).

### Technical Notes

- This story is the bug fix. Trace: J-OPS-002.
- Closes RCA root causes B + C + D structurally for Job kind. Root cause A (no settle
  window for Service start) remains open and is documented in `wave-decisions.md`.

---

## US-03: alloc status surfaces kind-aware semantics

### Problem

Ana submits the coinflip Job, sees the Failed verdict from the streaming submit, and
wants to confirm the details after the fact. She runs `overdrive alloc status --job
coinflip`. Today she sees a generic `Running` row regardless of whether the workload
actually succeeded or failed; the per-attempt exit codes are not surfaced. She wants
kind-aware semantics: for a Service, replica count + restarts + uptime; for a Job,
verdict + per-attempt exit codes.

### Who

- Overdrive platform engineer (Ana) | inspecting workloads after submit | wants the
  post-hoc view to match the kind's semantics.

### Solution

`AllocStatusRow.kind` denormalised at write time; CLI render branches on kind. Job
kind shows a `Verdict` line (`Succeeded` / `Failed (backoff exhausted)` / `In progress`)
and a per-attempt table with an `Exit` column. Service kind shows replicas + restarts
+ uptime, no Exit column. Schedule kind shows cron + deferral notice (this feature).

### Domain Examples

#### 1: Service inspection — Ana checks `payments` after 42s of uptime

`overdrive alloc status --job payments` outputs (Service render branch):
```
Job:    payments    (kind: Service)
Spec:   sha256:a4c1...e9
Replicas (desired/running): 1/1
Alloc                  State    Restarts  Since
---------------------- -------- --------- ----------
payments-0             Running  0         00:00:42.1
```

#### 2: Failed Job inspection — Ana checks `coinflip` after backoff exhausted

`overdrive alloc status --job coinflip` outputs (Job render branch):
```
Job:      coinflip    (kind: Job)
Spec:     sha256:b7f2...3a
Verdict:  Failed (backoff exhausted)
Attempt  State       Exit  Started               Duration
-------  ----------  ----  --------------------  --------
1        Failed      1     2026-05-09T14:27:02Z  0.2s
2        Failed      1     2026-05-09T14:27:03Z  0.2s
3        Failed      1     2026-05-09T14:27:05Z  0.3s
Last stderr (alloc coinflip-3, last 3 lines):
  ERROR
```

#### 3: In-progress Job — Ana checks `long-import` 2 minutes after submit

`overdrive alloc status --job long-import` outputs:
```
Job:      long-import    (kind: Job)
Spec:     sha256:e2a3...11
Verdict:  In progress (no terminal yet)
Attempt  State       Exit  Started               Duration
-------  ----------  ----  --------------------  --------
1        Running     —     2026-05-09T14:27:02Z  00:02:13
```

### UAT Scenarios (BDD)

#### Scenario: Service alloc status shows replicas and restarts, never an Exit column

```
Given a Service "payments" running stably with 1 of 1 replicas
When the operator runs "overdrive alloc status --job payments"
Then the output contains "kind: Service"
And the output contains "Replicas (desired/running): 1/1"
And the output's per-alloc table contains NO column named "Exit"
```

#### Scenario: Job alloc status (Failed) shows verdict, attempts, exit codes, stderr

```
Given a Job "coinflip" whose three attempts all exited 1 and the reconciler emitted BackoffExhausted
When the operator runs "overdrive alloc status --job coinflip"
Then the output contains "kind: Job"
And the output contains "Verdict: Failed (backoff exhausted)"
And the per-attempt table has columns Attempt, State, Exit, Started, Duration
And every Failed attempt row has Exit "1"
And the output includes the stderr tail of the last attempt
```

#### Scenario: Job alloc status (Succeeded) shows Verdict Succeeded with Exit 0

```
Given a Job "coinflip" whose first attempt exited 0 within 1.2 seconds
When the operator runs "overdrive alloc status --job coinflip"
Then the output contains "Verdict: Succeeded"
And the attempts table contains exactly one row with State "Succeeded" and Exit "0"
```

#### Scenario: Job alloc status NEVER renders Service phrasing

```
Given any Job-kind workload at any state
When the operator runs "overdrive alloc status --job <id>"
Then no line of output contains the substring "is running with"
```

### Acceptance Criteria

- [ ] `AllocStatusRow.kind` is denormalised at write time from the originally-
      submitted spec's kind.
- [ ] CLI alloc status branches on kind: Service / Job / Schedule renderers.
- [ ] Service render has no Exit column.
- [ ] Job render has Verdict header line + per-attempt table with Exit column.
- [ ] Job render shows stderr tail (last 5 lines) for Failed attempts.
- [ ] Anti-scenario: no Job alloc status output contains "is running with".

### Outcome KPIs

- **Who**: Overdrive platform engineers inspecting Failed Jobs.
- **Does what**: correctly identify exit code from `alloc status` output.
- **By how much**: ≥95% of operators in a usability check correctly state the exit
  code of a Failed Job from the alloc status output.
- **Measured by**: usability check (small sample, 5–10 operators) before/after this
  slice; stretch — automated parsing-from-fixtures test.
- **Baseline**: 0% — the current alloc status output does not surface per-attempt
  exit codes for Job kind, because the kind does not exist yet.

### Technical Notes

- User's explicit framing journey. Trace: J-OPS-003.
- Phase 1 has no surviving rows from before this feature; no migration needed.

---

## US-04: Service preserves existing semantics with kind-aware vocabulary

### Problem

Ana has existing Service workflows (long-running `/bin/sleep 3600`-style binaries
used in tests). She wants the kind discriminator feature to preserve their behaviour
exactly — same streaming `ConvergedRunning` semantics, same alloc status shape — with
only the rendered vocabulary updated to "Service" instead of "Job".

### Who

- Overdrive platform engineer (Ana) | maintaining existing Service workflows | wants
  no behavioural regression.

### Solution

The `format_running_summary` render function changes its output prefix from "Job" to
"Service" and replaces the literal `"live"` with a measured duration. Existing
integration tests for long-running workloads are migrated to the `[service]` shape;
their assertions update to expect "Service" not "Job".

### Domain Examples

#### 1: Long-running test fixture migration

The existing `streaming_submit_happy_path.rs` test submits a `/bin/sleep 3600` spec.
After this slice, it submits a `[service]` shape and asserts the output contains
"Service '...' is running with 1/1 replicas (took <duration>)" with a measured
duration.

#### 2: A real Service stabilises in 1.4s

```
Service 'payments' is running with 1/1 replicas (took 1.4s)
```
The duration is `clock.now() - submit_start_at`, never the literal `"live"`.

#### 3: A Service that crashes within the streaming window — current behaviour preserved

The existing `ConvergedFailed` arm fires (no change in shape, only vocabulary):
```
Service 'payments' failed to stabilise.
  alloc 'payments-0' exited with code 1 within 0.2s of start.
  Restart attempts: 1 of 5. Next attempt in ~0.5s.
```

### UAT Scenarios (BDD)

#### Scenario: Existing happy-path Service tests pass with renamed vocabulary

```
Given the existing streaming_submit_happy_path test fixture migrated to [service]
When the test runs against the live control plane
Then the test passes
And the rendered string contains "Service" not "Job"
And the rendered duration is a measured value, not "live"
```

#### Scenario: The literal "live" is absent from production source

```
Given the codebase after this feature lands
When a grep is run for the literal "live" in CLI render code
Then no production source-line contains it as a render literal
```

#### Scenario: A Service exit during stability window emits ConvergedFailed (vocabulary change only)

```
Given a Service whose process exits within the stability window
When the streaming subscriber observes the exit
Then the CLI prints "Service '<name>' failed to stabilise."
And the line continues to name attempts and next-retry timing
```

### Acceptance Criteria

- [ ] `format_running_summary` outputs "Service" prefix on the Service code path.
- [ ] No call site passes the literal `"live"` as a duration.
- [ ] All existing integration tests for long-running workloads pass after migration.
- [ ] Grep gate / dst-lint rule rejects future re-introduction of `"live"` literal.

### Outcome KPIs

- **Who**: existing Service-shaped integration tests.
- **Does what**: continue to pass after the kind rename.
- **By how much**: 100% of existing Service tests pass with the migrated fixture.
- **Measured by**: CI run.
- **Baseline**: 100% pass today on the legacy shape.

### Technical Notes

- Closes RCA root cause D (`"live"` literal). Trace: J-OPS-002.
- Companion to US-02 — both ship "vocabulary is honest about kind".

---

## US-05: Schedule parses with honest deferral

### Problem

Ana wants to declare a recurring backup. She writes `[job] + [schedule]` in TOML.
Today the parser does not recognise `[schedule]`, so she gets a generic deserialize
error. She wants the platform to (a) accept her syntactic intent, (b) tell her
honestly that execution is not yet implemented, and (c) point her to a tracking issue
so she knows when execution arrives.

### Who

- Overdrive platform engineer (Ana) | planning recurring jobs | wants the spec
  validated and the deferral named.

### Solution

Parser support for `[job] + [schedule]` with a string `cron` field. CLI submit echo
prints "Schedule registered" plus a NOTE about deferred execution with a tracking
URL. CLI alloc status renders cron + the same deferral URL. The URL is a single CLI
config constant.

### Domain Examples

#### 1: Valid Schedule spec — Ana submits `nightly-backup.toml`

```
$ overdrive job submit ./nightly-backup.toml
Submitting schedule 'nightly-backup' (kind=Schedule)
Spec digest: sha256:c9e1...77
Endpoint:    https://127.0.0.1:7001/
Schedule registered.

NOTE: schedule execution is not yet implemented in this Phase 1 slice.
      The spec has been validated and persisted as intent; no Job runs
      will be spawned automatically.
      Tracking: https://github.com/overdrive-sh/overdrive/issues/166
```
CLI process exit: 0.

#### 2: Invalid composition — `[schedule]` without `[job]`

```
$ overdrive job submit ./bad.toml
Error: spec at './bad.toml' contains [schedule] without [job].
       [schedule] is only valid alongside [job], not [service] or alone.
```
CLI process exit: non-zero.

#### 3: alloc status reflects the deferral consistently

```
$ overdrive alloc status --job nightly-backup
Job:    nightly-backup    (kind: Schedule)
Spec:   sha256:c9e1...77
Cron:   0 2 * * *

No allocations have been spawned yet.

Reason: Schedule execution is not yet implemented (issue #166).
```

### UAT Scenarios (BDD)

#### Scenario: A Scheduled Job spec is recognised by [job] + [schedule] co-presence

```
Given a TOML file with [job], [schedule] (cron = "0 2 * * *"), [exec], [resources]
When the parser reads the file
Then a Schedule-kind workload spec is constructed
And the cron expression is captured as a string field
```

#### Scenario: A spec with [schedule] but no [job] is rejected

```
Given a TOML file containing [schedule] without [job]
When the operator runs "overdrive job submit ./bad.toml"
Then the CLI prints an error naming "[schedule] is only valid alongside [job]"
And the CLI exits with a non-zero status code
```

#### Scenario: Submit echoes "registered" plus a deferral note with tracking URL

```
Given a Scheduled Job spec at "./nightly-backup.toml"
When the operator runs "overdrive job submit ./nightly-backup.toml"
Then the CLI prints "Submitting schedule 'nightly-backup' (kind=Schedule)"
And the CLI prints "Schedule registered."
And the CLI prints a NOTE saying execution is not yet implemented
And the NOTE includes the tracking issue URL
And the CLI exits with status 0
```

#### Scenario: alloc status for a Schedule names the deferral with the same URL

```
Given a registered Scheduled Job whose execution is deferred
When the operator runs "overdrive alloc status --job <id>"
Then the output contains "kind: Schedule"
And the output contains the cron expression
And the output's deferral URL byte-matches the URL printed by the submit echo
```

### Acceptance Criteria

- [ ] Parser recognises `[job] + [schedule]` co-presence as Schedule kind.
- [ ] Required cron field is enforced.
- [ ] `[schedule]` without `[job]` rejected.
- [ ] `[schedule]` with `[service]` rejected.
- [ ] Submit echo prints "Schedule registered" + deferral NOTE + tracking URL.
- [ ] alloc status renders cron + deferral with same tracking URL.
- [ ] The tracking URL is a single CLI config constant (single source of truth).

### Outcome KPIs

- **Who**: Overdrive platform engineers planning recurring workloads.
- **Does what**: receive consistent deferral messaging across submit and alloc status.
- **By how much**: 100% of Schedule submits produce identical deferral URLs across
  submit echo and alloc status output (byte-equality, asserted in integration test).
- **Measured by**: integration test that runs both commands and compares the URL
  string.
- **Baseline**: not applicable — feature did not exist.

### Technical Notes

- Depends on a deferral the user must approve. See `wave-decisions.md` § "Deferrals
  requiring user approval". Until the GH issue exists, the constant uses a placeholder
  the orchestrator updates post-approval. Slice CANNOT land with placeholder visible
  to operators.
- Trace: J-OPS-002.

---

## US-06: Anti-pattern grep gate for `"live"` literal (technical task)

### Problem

The bug under audit included a hard-coded `"live"` literal at
`crates/overdrive-cli/src/commands/job.rs:504` that masqueraded as a duration. After
this feature lands, no production code path produces that literal — but a future
contributor could re-introduce it inadvertently. We want a CI gate that fails the
build if the literal is reintroduced.

### Who

- Overdrive platform engineer maintaining the CLI render layer.

### Solution

A grep gate in the workspace's existing CI pipeline (or an `xtask` lint, or a
`dst-lint`-style scanner) that fails if the literal string `"live"` appears in any
`.rs` file under `crates/overdrive-cli/src/render*` or
`crates/overdrive-cli/src/commands/*` as a string literal passed to a duration-
formatting function. Exception: comments and docstrings are allowed to mention the
historical literal for context.

### Linked user story

Implements the regression guard for **US-02** (Job submit terminal verdict). Required
per the LeanUX template's Technical Task type.

### Acceptance Criteria

- [ ] CI fails when a `"live"` literal is added to a render or command source file.
- [ ] CI passes when the literal appears in a comment / docstring / RCA reference.
- [ ] A regression test deliberately introduces the literal and asserts CI fails.

### Technical Notes

- Implemented as part of Slice 01.

---

## US-08: Service listener spec shape (port, protocol, optional VIP)

### Changed Assumptions

This story was added to the DISCUSS wave on 2026-05-10 to fold in
[overdrive-sh/overdrive#164](https://github.com/overdrive-sh/overdrive/issues/164)
(service listener spec shape). User explicitly approved the fold-in. The prior
wave-decisions.md handoff scope (5 slices, 7 stories) is superseded — see
`wave-decisions.md` § "Fold-in of GH #164". The runtime VIP allocator behavior
when `vip = None` is tracked as
[overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167)
(approved 2026-05-09); this slice ships only the spec shape, which is forward-
compatible regardless of how #167 lands.

### Problem

Ana, Overdrive platform engineer, has a `payments` Service that listens on TCP
port 8080 (HTTP) and UDP port 8081 (a sidecar metrics shipper). Today the
`[service]` block carries no listener fields — there is no way to tell the
platform what protocols and ports her workload accepts. She finds it impossible
to declare per-listener `(port, protocol)` pairs in the spec, so the dataplane
has no way to map traffic to her workload by protocol. She wants to write
`[[listener]]` blocks under her Service spec and have the parser validate them
at submit time, so the platform's view matches what her workload actually
serves. She also wants to optionally pin a VIP — when she omits it, the platform
should allocate one (per #167); when she sets it, the platform should honour it.

### Who

- Overdrive platform engineer (Ana) | declaring Service workloads with
  protocol/port-specific listeners | wants the spec to carry listener triples
  and have them surface honestly across submit and `alloc status`.

### Solution

Extend the `[service]` discriminator to accept a top-level array-of-tables
`[[listener]]` carrying three fields: `port: NonZeroU16`, `protocol: Proto`
(case-insensitive `tcp`/`udp`), and `vip: Option<ServiceVip>` (IPv4). The parser
enforces uniqueness on `(vip, port, protocol)` triples within a Service, requires
at least one listener, rejects unsupported protocols, and reuses the existing
`Proto` newtype from `overdrive-core`. The CLI submit echo and `alloc status`
render gain a Listeners section. OpenAPI roundtrip stays clean via
`utoipa::ToSchema` derives. The runtime behaviour for `vip = None` (allocator
vs. admission rejection) is OUT OF SCOPE here and tracked at #167.

### Domain Examples

#### 1: Happy path — Ana writes a Service spec for `frontend` with two listeners

Ana has a frontend that serves HTTP on TCP/8080 and a UDP heartbeat on 8081.
She writes:
```toml
[service]
id = "frontend"
replicas = 2

[[listener]]
port     = 8080
protocol = "tcp"
vip      = "10.0.0.1"

[[listener]]
port     = 8081
protocol = "udp"

[exec]
command = "/usr/local/bin/frontend"
[resources]
cpu_milli = 500
memory_bytes = 268435456
```
Parser produces `WorkloadSpec::Service { id: "frontend", listeners: [
  Listener { port: 8080, protocol: Proto::Tcp, vip: Some(10.0.0.1) },
  Listener { port: 8081, protocol: Proto::Udp, vip: None },
], ... }`. CLI submit echo prints two listener lines:
```
Listeners:
  10.0.0.1:8080/tcp
  (vip: pending allocation — see #167):8081/udp
```

#### 2: Edge case — Ana writes a Service with one listener, case-insensitive protocol

Ana has a `payments` Service serving HTTPS on TCP/8443:
```toml
[service]
id = "payments"
[[listener]]
port     = 8443
protocol = "TCP"        # any case is accepted
[exec]
command = "/usr/local/bin/payments-server"
[resources]
cpu_milli = 500
memory_bytes = 268435456
```
Parser canonicalises to `Proto::Tcp`. The submit echo's listener line shows
`(vip: pending allocation — see #167):8443/tcp` — protocol always rendered
lowercase.

#### 3: Error — Ana accidentally declares duplicate listeners

Ana copy-pastes a listener block and forgets to change the port:
```toml
[service]
id = "broken"
[[listener]]
port     = 8080
protocol = "tcp"
[[listener]]
port     = 8080
protocol = "tcp"
[exec] ...
[resources] ...
```
CLI prints:
```
Error: spec at './broken.toml' declares two [[listener]] blocks with the
       same (vip, port, protocol) triple: (none, 8080, tcp).
       Each listener within a Service must have a distinct triple.
```
Exit code: non-zero.

### UAT Scenarios (BDD)

#### Scenario: A Service with two valid listeners parses with both triples preserved in declaration order

```
Given a TOML file at "./frontend.toml" with [service] id="frontend" and two [[listener]] blocks (8080/tcp/10.0.0.1 then 8081/udp/none)
When the parser reads the file
Then a Service-kind workload spec is constructed
And the spec carries listeners [(10.0.0.1, 8080, tcp), (none, 8081, udp)] in declaration order
```

#### Scenario: Protocol parsing is case-insensitive and canonicalises to lowercase

```
Given a TOML file with a [[listener]] whose protocol is "TCP"
When the parser reads the file
Then the parsed listener's protocol equals Proto::Tcp
And every CLI surface that renders the protocol prints "tcp" (lowercase)
```

#### Scenario: A Service with zero listeners is rejected with named guidance

```
Given a TOML file with [service] but no [[listener]] blocks
When the operator runs "overdrive job submit ./no-listener.toml"
Then the CLI prints an error stating "a [service] requires at least one [[listener]] block"
And the CLI exits with a non-zero status code
```

#### Scenario: A duplicate (vip, port, protocol) triple is rejected with named guidance

```
Given a TOML file with two [[listener]] blocks both naming (none, 8080, tcp)
When the operator runs "overdrive job submit ./duplicate.toml"
Then the CLI prints an error naming the duplicate triple
And the CLI exits with a non-zero status code
```

#### Scenario: An unsupported protocol value is rejected (sctp, icmp, empty)

```
Given a TOML file with a [[listener]] whose protocol is "sctp"
When the operator runs "overdrive job submit ./bad-proto.toml"
Then the CLI prints an error naming "sctp" as an unsupported protocol
And the error names the supported set as "tcp, udp"
And the CLI exits with a non-zero status code
```

#### Scenario: port = 0 is rejected

```
Given a TOML file with a [[listener]] whose port is 0
When the operator runs "overdrive job submit ./bad-port.toml"
Then the CLI prints an error naming "port must be in 1..=65535"
And the CLI exits with a non-zero status code
```

#### Scenario: Submit echo surfaces every listener with pinned-or-pending VIP

```
Given a Service spec with one pinned-VIP listener and one None-VIP listener
When the operator runs "overdrive job submit ./mixed-vip.toml"
Then the CLI submit echo includes a "Listeners:" section
And the pinned-VIP listener is rendered as "<vip>:<port>/<protocol>"
And the None-VIP listener is rendered as "(vip: pending allocation — see #167):<port>/<protocol>"
```

#### Scenario: alloc status renders a Listeners section for a Service

```
Given a Service "frontend" with two listeners (one pinned, one pending)
When the operator runs "overdrive alloc status --job frontend"
Then the output contains "Listeners:"
And both listeners appear with their (vip-or-pending, port, protocol) triple
And every listener line's protocol is rendered lowercase
```

#### Scenario: A JobSpecInput round-trips bit-equivalently through TOML, JSON, and Job

```
Given an arbitrary JobSpecInput with N valid listener triples
When it is serialised to TOML, parsed back, converted to a Job aggregate, and converted back via JobSpecInput::from(&Job)
Then the resulting JobSpecInput equals the original
```

#### Scenario: OpenAPI roundtrip passes for the new listener types

```
Given the Listener and ServiceVip newtypes derive utoipa::ToSchema
When the operator runs "cargo openapi-gen" and "cargo openapi-check"
Then both commands exit with status 0
And the generated schema includes Listener with port, protocol, and optional vip fields
```

### Acceptance Criteria

- [ ] `[[listener]]` array-of-tables parses under `[service]` with `port`,
      `protocol`, and optional `vip` fields.
- [ ] `port` is `NonZeroU16` (parser rejects 0).
- [ ] `protocol` parses case-insensitively via the existing
      `overdrive-core::Proto` newtype; canonical render is lowercase.
- [ ] `vip` is `Option<ServiceVip>`; absent value is `None`, present value is
      validated as IPv4 syntax.
- [ ] Parser rejects a Service with zero `[[listener]]` blocks.
- [ ] Parser rejects two listeners with the same `(vip, port, protocol)` triple
      (when both `vip` are `None`, comparison is on `(port, protocol)` only).
- [ ] Parser rejects unsupported protocols (`sctp`, `icmp`, empty string,
      anything not `tcp`/`udp`) with named guidance.
- [ ] CLI submit echo includes a `Listeners:` section, one line per listener,
      with `(vip-or-pending, port, protocol)`. Pending VIPs render as
      `(vip: pending allocation — see #167)`.
- [ ] `alloc status --job <id>` for a Service renders a `Listeners:` section
      mirroring submit echo semantics.
- [ ] `JobSpecInput`, `Job`, and the listener types derive
      `Serialize + Deserialize + utoipa::ToSchema`; `cargo openapi-gen` /
      `cargo openapi-check` both pass.
- [ ] Property test: every valid `JobSpecInput` round-trips bit-equivalent
      through TOML / JSON / `Job::from_spec` / `JobSpecInput::from(&Job)`.

### Outcome KPIs

- **Who**: Overdrive platform engineers writing Service specs.
- **Does what**: declare per-listener `(port, protocol, vip?)` and trust the
  CLI's submit-and-status surfaces to round-trip them byte-identically.
- **By how much**: 100% of Service submits with explicitly pinned VIPs see the
  byte-identical `(vip, port, protocol)` triple in `alloc status`. 100% of
  invalid composition cases (zero listeners, duplicate triple, unsupported
  protocol, port=0) are rejected with named guidance.
- **Measured by**: integration test that submits 100 Service specs with pinned
  VIPs and asserts byte-equality between submit echo and `alloc status`
  listener rendering; parser unit tests for the rejection paths.
- **Baseline**: 0% — the listener fields do not exist on the Service spec
  today.

### Technical Notes

- Reuses `overdrive-core::Proto` — no second copy of the newtype. Spec layer
  imports the kernel-side enum the dataplane already uses.
- Field name is `protocol` (not `proto`) to match Kubernetes terminology and
  improve operator readability.
- Section name is `[[listener]]` — NOT `[[backend]]`, which would collide with
  the dataplane's existing destination-address `Backend` type.
- Runtime behaviour for `vip = None` is OUT OF SCOPE — tracked at #167. The
  spec layer's job is to carry the field shape forward. Whichever way the
  runtime decision lands (admission-time rejection vs. allocator), the spec
  field stays `Option`-shaped.
- Trace: J-OPS-002 (primary — submit and trust what the CLI tells me),
  J-OPS-003 (secondary — convergence honesty extends to listener shape).
- JTBD analysis skipped per `wave-decisions.md` § "JTBD traceability"; the
  motivation is downstream of J-OPS-002's "honest about what it does and does
  not know" clause and J-OPS-003's listener-aware convergence semantics.

---

## US-07: Migrate `examples/coinflip.toml` to `[job]` shape

### Problem

The current `examples/coinflip.toml` has a flat shape (no kind discriminator). This
file is the canonical reproduction of the bug. After the parser changes in Slice 01,
the file must be migrated to the `[job]` shape so it continues to be the bug
reproduction (and to demonstrate the fix).

### Who

- Overdrive platform engineer (Ana) using the example to reproduce or learn from.

### Solution

A single-cut migration of `examples/coinflip.toml` to:
```toml
[job]
id = "coinflip"

[exec]
command = "/bin/bash"
args = [
    "-c",
    "if (( RANDOM % 2 )); then echo SUCCESS; exit 0; else echo ERROR >&2; exit 1; fi",
]

[resources]
cpu_milli = 100
memory_bytes = 67108864
```
The `replicas = 1` line is dropped (Job kind has its own replicas/parallelism shape per
research R1 — for this single-shot use-case, defaults).

### Acceptance Criteria

- [ ] `examples/coinflip.toml` is rewritten to the `[job]` shape.
- [ ] The file parses successfully under the new parser (Slice 01).
- [ ] Submitting the file under Slice 02 produces the kind-aware terminal verdict.

### Technical Notes

- Single-cut migration per `feedback_single_cut_greenfield_migrations.md`. No flat-
  shape compatibility shim.
