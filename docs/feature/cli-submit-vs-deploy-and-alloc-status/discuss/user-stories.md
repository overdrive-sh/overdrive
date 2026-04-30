<!-- markdownlint-disable MD024 -->

# User Stories — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DISCUSS / Phase 3
**Owner**: Luna (`nw-product-owner`)
**Date**: 2026-04-30

Every story traces to the validated job from
`diverge/job-analysis.md` ("Reduce the time and uncertainty between
declaring intent and knowing whether the platform converged on it"),
addressing one or more of the 6 ODI outcomes.

## System Constraints (cross-cutting)

These constraints apply to every story below.

- **Phase 1 single-node.** No multi-node placement, no taints. The
  local node is implicit.
- **Reconciler purity.** §18 + ADR-0023. The streaming endpoint is a
  consumer of ObservationStore rows, not a producer that blocks the
  reconciler tick. No I/O inside `reconcile()`.
- **Intent / Observation split.** Submit writes intent through
  IntentStore; convergence is observed through ObservationStore.
- **Shared types.** Per ADR-0014. CLI and server share the typed
  request/response surface in `overdrive-control-plane::api`. New
  types (`SubmitEvent`, snapshot extension) live there.
- **Error shape.** Per ADR-0015. Existing `ErrorBody` covers
  validation/not-found/conflict/internal; the streaming endpoint
  reuses it for the JSON-ack path and for HTTP-level errors that
  precede streaming (e.g. 400 on bad TOML). Streaming-protocol-level
  failures (mid-stream) become NDJSON `ConvergedFailed` events, not
  `ErrorBody`.
- **Exit-code contract.** 0 = converged Running. 1 = converged
  Failed (any cause: driver, backoff exhausted, server timeout). 2 =
  client-side / transport / server-validation error per ADR-0015.
  64–78 (sysexits.h range) reserved; not used.
- **NDJSON over SSE (Key Decision).** See `wave-decisions.md`. CLI
  is the only consumer; submit is one-shot, not a long-lived feed;
  serde_json + reqwest line-delimited consumption is mature; OpenAPI
  describes NDJSON natively.

---

## US-01 — Streaming submit: happy path

### Problem

Ana, an Overdrive platform engineer running her inner-loop
edit-submit-observe-fix cycle, finishes editing `payments.toml` and
wants to know within seconds whether the platform converged on the
spec. Today, `overdrive job submit` returns `Accepted.` immediately
and forces her to invent a polling loop against `alloc status` to
discover whether anything is actually running. She finds it
context-switching to the polling step every time, and not honest
about what the platform has done.

### Who

- Senior platform / SRE engineer | inner-loop edit-submit-observe-fix
  cycle | one verb, one terminal, one decision.

### Solution

`overdrive job submit ./payments.toml` streams the lifecycle
reconciler's convergence inline as NDJSON. CLI exits 0 once the
allocation reaches Running. Operator sees lifecycle transitions in
real time, gets a one-line summary on success, and the verb tells
the truth about whether the workload is running.

### Domain Examples

#### 1: Happy path — Ana submits a fresh `payments-v2` spec

Ana runs `overdrive job submit ./payments.toml`. The TOML declares
`exec.command = "/usr/local/bin/payments"` (a real binary on her
dev host). First NDJSON line arrives in 80 ms carrying spec digest
`sha256:7f3a9b12...` and intent-key `payments-v2`. Three lifecycle
lines stream as the reconciler converges: pending/scheduling,
pending/starting, running/pid 12345. Total wall clock 1.4 s. CLI
prints `Job 'payments-v2' is running with 1/1 replicas (took 1.4s).`
and exits 0.

#### 2: Re-submit unchanged spec — idempotency

Ana re-runs `overdrive job submit ./payments.toml` an hour later
without editing. Server's `IntentStore::put_if_absent` returns
`IdempotencyOutcome::Unchanged`. First NDJSON event is `Accepted`
with `outcome: Unchanged`. The allocation is still Running from the
prior submit; reconciler emits no new transitions; stream emits a
single `ConvergedRunning { alloc_id: a1b2c3, started_at: ... }` and
closes. CLI exits 0.

#### 3: Slow convergence inside the budget — backoff window

Ana submits a spec whose binary takes 4 seconds to start (for
example, a Python interpreter loading a heavy import graph).
ProcessDriver reports `pending` for the first 4 s, then `running`.
Stream emits 5 lines total. Operator's terminal is occupied for ~5
seconds; CLI exits 0. The first NDJSON line still landed at 90 ms,
so the operator never wondered whether the CLI hung.

### UAT Scenarios (BDD)

#### Scenario: Streaming submit on a healthy spec exits zero with a summary

Given Ana runs `overdrive job submit ./payments.toml` from an interactive terminal
And the spec's binary exists and runs cleanly
When the lifecycle reconciler converges the allocation to Running
Then the CLI prints lifecycle transitions inline in real time
And the CLI prints a one-line summary `Job 'payments-v2' is running with 1/1 replicas (took D)`
And the CLI exits with code 0

#### Scenario: First NDJSON line lands within 200 ms (emotional contract)

Given Ana runs streaming submit on a healthy local control plane
When the server commits the spec to the IntentStore
Then the first NDJSON line is delivered to the CLI within 200 ms p95

#### Scenario: Idempotent re-submission of an unchanged spec exits zero

Given a job `payments-v2` is already running from a prior submit
When Ana re-runs `overdrive job submit ./payments.toml` with the byte-identical spec
Then the first NDJSON line is `Accepted` with `outcome: Unchanged`
And the stream emits a single `ConvergedRunning` event referencing the existing allocation
And the CLI exits with code 0

### Acceptance Criteria

- [ ] Streaming submit on a healthy spec exits 0 with summary line.
- [ ] First NDJSON line lands within 200 ms p95 on a healthy local
  control plane.
- [ ] Re-submit of unchanged spec exits 0 with
  `outcome: Unchanged` first event.
- [ ] Lifecycle transitions stream as NDJSON lines with structured
  fields per the journey YAML.

### Outcome KPIs

- **Who**: Ana (and operators in her role).
- **Does what**: completes the inner-loop edit-submit-observe-fix
  cycle without typing a second command.
- **By how much**: time-to-knowing-convergence-result drops from
  "indefinite (depends on operator polling)" to "≤ convergence
  wall-clock + 200 ms first-event budget."
- **Measured by**: timestamp delta between CLI POST and CLI exit-0
  (success path) on the canonical happy-path scenario.
- **Baseline**: today, the operator exits the `submit` verb at
  `Accepted.` and decides separately to poll; baseline measurement
  is "operator's polling loop interval" — typically 1–5 s and
  human-controlled.

### Technical Notes

- Trace: addresses ODI outcomes 1, 4 (time-to-know convergence;
  effort to observe transitions without external tools).

---

## US-02 — Streaming submit: failure path (broken-binary regression)

### Problem

The same Ana, the same inner loop. She edits `payments.toml` to
point at a non-existing binary path (deliberately, while debugging,
or accidentally during a refactor). Today, `overdrive job submit`
returns `Accepted.` and exits 0 — and she has no idea the workload
is silently failing in a backoff loop. She has to remember to run
`alloc status`, and even then today's output (`Allocations: 1`)
doesn't tell her what failed. This is the user's ACTUAL session and
the regression-target case.

### Who

- Senior platform / SRE engineer | currently in a tight debug loop
  with a misbehaving spec | needs to fail fast with the cause named.

### Solution

`overdrive job submit ./payments.toml` streams the lifecycle
reconciler's convergence; when the driver fails to start the binary
and the lifecycle reconciler exhausts its restart budget, the stream
closes with a `ConvergedFailed` event. CLI prints a structured
`Error:` block naming the reason, the verbatim driver error, and a
reproducer command, and exits 1.

### Domain Examples

#### 1: Binary not found — ENOENT

Ana edits `payments.toml` so `exec.command =
"/usr/local/bin/payments"` (no such binary). She runs
`overdrive job submit ./payments.toml`. First NDJSON line:
`Accepted`. Reconciler emits `pending starting (attempt 1)`;
ProcessDriver returns ENOENT-class error; reconciler emits `failed
driver: stat /usr/local/bin/payments: no such file or directory`;
backoff window 5 s; reconciler retries 4 more times, each failing
identically. After 5 attempts, restart budget exhausted; stream
closes with `ConvergedFailed { terminal_reason: backoff_exhausted,
error: "stat /usr/local/bin/payments: no such file or directory"
}`. CLI prints:

```
Error: job 'payments-v2' did not converge to running.
  reason: driver start failed (binary not found)
  last-event: stat /usr/local/bin/payments: no such file or directory
  reproducer: overdrive alloc status --job payments-v2

Hint: fix the spec's `exec.command` path and re-run.
```

CLI exits 1.

#### 2: Permission denied — EACCES

Ana points the spec at a binary that exists but lacks execute
permission. Reconciler attempts start; ProcessDriver returns
EACCES-class error. Same shape as Example 1, with
`error: "exec /usr/local/bin/payments: permission denied"`. CLI
exits 1.

#### 3: Server-side wall-clock cap

Ana points the spec at a binary that takes 90 seconds to start
(pathological case). The server-side wall-clock cap is configured
at 60 seconds. After 60 s, the stream closes with `ConvergedFailed
{ terminal_reason: timeout, error: "did not converge in 60s" }`.
CLI prints `did not converge in 60s` and exits 1. The allocation is
still Pending in `alloc status`; the operator can decide to wait
longer (raising the cap) or fix the spec.

### UAT Scenarios (BDD)

#### Scenario: Convergence to Failed exits non-zero with a structured error

Given Ana runs `overdrive job submit ./payments.toml` and the spec's binary does not exist
When the lifecycle reconciler exhausts its restart budget
Then the stream closes with `ConvergedFailed`
And the CLI prints an `Error:` block including reason, verbatim driver error, and reproducer command
And the CLI exits with code 1

#### Scenario: Same failure reason in stream and snapshot

Given a streaming submit emitted `ConvergedFailed` with reason R for allocation A
When Ana subsequently runs `overdrive alloc status --job payments-v2`
Then the snapshot's `last_transition.reason` for allocation A equals R verbatim

#### Scenario: Server wall-clock cap surfaces as Failed terminal event

Given Ana runs streaming submit and convergence does not complete within the server-side cap
When the cap is exceeded
Then the server emits `ConvergedFailed` with `terminal_reason: timeout`
And the CLI exits with code 1

### Acceptance Criteria

- [ ] Streaming submit on a broken-binary spec exits 1 with
  structured `Error:` block.
- [ ] The verbatim driver error appears in the CLI output.
- [ ] The CLI output names a reproducer command (`alloc status
  --job ...`).
- [ ] Server wall-clock cap exceeded produces `ConvergedFailed
  { terminal_reason: timeout, ... }`; CLI exits 1.
- [ ] The `reason` string in `ConvergedFailed` for a given
  allocation equals the `last_transition.reason` rendered by `alloc
  status` for the same allocation, byte-for-byte.

### Outcome KPIs

- **Who**: Ana / debugging operator.
- **Does what**: identifies the cause of a non-converging deploy
  inline, without pivoting to `journalctl` or
  `systemctl status` or `cat /sys/fs/cgroup/...`.
- **By how much**: time-to-identify-failure-reason drops from
  "indefinite (operator must poll, then re-run a separate diagnostic
  command)" to "≤ convergence-or-cap wall clock."
- **Measured by**: regression-target test asserts exit code 1, the
  verbatim driver error is in stdout, and reason strings match
  across stream and snapshot. Boolean pass/fail; not a rate.
- **Baseline**: today, exit 0 from submit on a broken-binary spec
  is a silent-accept; the operator only discovers failure via
  separate command + manual log inspection.

### Technical Notes

- Trace: addresses ODI outcomes 2 (likelihood of silent-accept), 3
  (time to identify reason), 6 (distinguish "not yet" from
  "failed").
- This is the **regression-target** story. If this fails the
  acceptance test, the feature has not delivered.

---

## US-03 — `--detach` for CI / automation

### Problem

Ana also runs `overdrive job submit` from CI (a GitHub Actions
workflow that submits a fresh build for soak testing). In that
context, holding the connection open for a multi-second convergence
window is wrong — the CI script's job is to commit intent and
return; observation belongs to a separate later job.

### Who

- CI / automation scripts | running in non-interactive contexts |
  need a one-shot intent commit with a JSON ack.

### Solution

`overdrive job submit ./payments.toml --detach` sends `Accept:
application/json`, gets the existing one-line JSON ack
(`{"spec_digest":"...", "intent_key":"...", "outcome":"..."}`), and
exits 0. No streaming machinery activated.

### Domain Examples

#### 1: GitHub Actions workflow

CI workflow runs `overdrive job submit ./payments.toml --detach`
inside a step. Stdout is captured to `submit.json`. The next step
runs `jq -r .spec_digest < submit.json` to grab the digest for a
later observability call. CLI exits 0 immediately.

#### 2: Bash script with explicit detach

Ana's local bash script submits 10 jobs in a loop, then waits for
all of them via `alloc status`. Each `submit --detach` call exits
in <100 ms (just the IntentStore commit + JSON write). The script
collects `intent_key`s into an array; a separate verification step
runs after the loop completes.

#### 3: Failure during `--detach` submit

Ana runs `submit --detach` against a control plane that's down. The
CLI's reqwest client returns a transport error (no HTTP response).
CLI prints the transport error and exits 2. No NDJSON involvement.

### UAT Scenarios (BDD)

#### Scenario: --detach produces single JSON ack and exits zero

Given Ana runs `overdrive job submit ./payments.toml --detach` interactively
When the server commits the spec
Then the CLI's request carries `Accept: application/json`
And stdout is a single JSON object with `spec_digest`, `intent_key`, `outcome`
And the CLI exits with code 0

#### Scenario: --detach overrides TTY default

Given Ana is on an interactive terminal
When she passes `--detach`
Then the CLI does not consume an NDJSON stream
And exits as soon as the JSON ack arrives

### Acceptance Criteria

- [ ] `--detach` flag sends `Accept: application/json` regardless of
  TTY state.
- [ ] Output is a single JSON object equivalent to today's response
  shape.
- [ ] CLI exits 0 on successful commit.
- [ ] CLI exits 2 on transport / server-validation error per
  ADR-0015 (no NDJSON path).

### Outcome KPIs

- **Who**: CI scripts and automation operators.
- **Does what**: scripts a fire-and-forget submit that never blocks
  on convergence.
- **By how much**: zero seconds spent waiting; CLI exits as soon as
  IntentStore commits.
- **Measured by**: `--detach` flag passes acceptance test; CLI exit
  time < 200 ms p95 on a healthy control plane.
- **Baseline**: today's default behaviour. (`--detach` preserves
  the current shape under an explicit flag rather than changing
  semantics.)

### Technical Notes

- Trace: addresses the dissenting case from the DIVERGE document —
  CI/automation use case is preserved as a first-class flag, not a
  workaround.
- Pairs with US-04 (auto-detach on pipe).

---

## US-04 — Auto-detach when stdout is piped

### Problem

A common operator pattern: `overdrive job submit ./job.toml | jq -r
.spec_digest` to extract the digest into another script. If submit
default-streams NDJSON, this pipeline breaks: jq receives multiple
JSON objects across multiple lines, not a single object. Forcing
the operator to remember `--detach` for every pipeline is friction
the platform should eliminate.

### Who

- Any operator using `submit` in a Unix pipeline | non-TTY stdout |
  expects single-JSON-object output.

### Solution

CLI calls `isatty(stdout)`. If stdout is NOT a TTY (piped,
redirected, or running in a CI context that doesn't allocate a
TTY), CLI sends `Accept: application/json` automatically. Same as
`--detach` semantics; no flag required.

### Domain Examples

#### 1: jq pipeline

Ana runs `overdrive job submit ./payments.toml | jq -r
.spec_digest`. CLI detects non-TTY stdout, sends `Accept:
application/json`, gets the JSON ack, jq extracts `.spec_digest`,
prints to terminal. CLI exits 0.

#### 2: File redirection

Ana runs `overdrive job submit ./payments.toml > /tmp/out.json`.
Same path: non-TTY stdout, JSON ack to file, exit 0.

#### 3: GitHub Actions step without explicit `--detach`

CI step runs `overdrive job submit ./payments.toml`. GitHub Actions
runners do not allocate a TTY for shell steps. CLI auto-detaches;
JSON ack lands; exit 0. Operator never had to remember `--detach`.

### UAT Scenarios (BDD)

#### Scenario: piping to jq returns a single JSON object

Given Ana runs `overdrive job submit ./payments.toml | jq -r .spec_digest`
When the CLI detects that stdout is not a TTY
Then the CLI sends `Accept: application/json`
And the jq output is a single line containing the digest
And the CLI exits with code 0

#### Scenario: redirection to file produces single JSON

Given Ana runs `overdrive job submit ./payments.toml > /tmp/out.json`
When the CLI detects that stdout is not a TTY
Then `/tmp/out.json` contains a single JSON object
And the CLI exits with code 0

### Acceptance Criteria

- [ ] CLI calls `isatty(stdout)` and sends
  `Accept: application/json` when stdout is not a TTY.
- [ ] `submit | jq -r .spec_digest` works without `--detach`.
- [ ] `submit > file.json` produces a single JSON object in the file.
- [ ] On a TTY without `--detach`, CLI defaults to NDJSON streaming
  (does not auto-detach).

### Outcome KPIs

- **Who**: operators using `submit` in pipelines.
- **Does what**: pipes/redirections work without the operator
  explicitly passing `--detach`.
- **By how much**: zero memorisation tax; pipelines that worked
  pre-feature continue to work post-feature with the same syntax.
- **Measured by**: acceptance tests for jq-pipe and file-redirect.
- **Baseline**: pipelines today receive the existing JSON ack
  (because submit doesn't stream); the auto-detach preserves that
  behaviour under a TTY-aware heuristic rather than changing it.

### Technical Notes

- Trace: addresses CI/automation friction; eliminates one of the
  two named risks of Option S in DIVERGE (the other being the
  long-lived HTTP request, which the server-side cap addresses).
- Pairs with US-03 (explicit `--detach` flag).

---

## US-05 — `alloc status` enrichment (snapshot density)

### Problem

Today's `overdrive alloc status --job payments` renders
`Allocations: 1` and nothing else. Even after streaming submit
lands, an operator inspecting an allocation post-deployment ("what
is this thing doing right now? did it crash since I last checked?
how close is the lifecycle reconciler to giving up?") needs a
denser snapshot. Today's output forces them to invent diagnostic
workflows out of band.

### Who

- Senior platform / SRE engineer doing second-day inspection | wants
  state, last transition, restart budget, error if any | one
  command, dense answer.

### Solution

`overdrive alloc status --job payments-v2` returns a typed snapshot
(`AllocStatusSnapshot` or extended `AllocStatusResponse`) carrying
state, resources, started timestamp, exit code, last-transition
block (`from`, `to`, `reason`, `source`, `at`), and restart budget
(used / max / exhausted). CLI renders the TUI mockup specified in
the journey.

### Domain Examples

#### 1: Running allocation

Ana runs `alloc status` against a healthy `payments-v2` job. Output
shows `STATE: Running`, `RESOURCES: 2000mCPU/4 GiB`, `STARTED:
2026-04-30T10:15:32Z`, `Last transition: ... reason: driver started
(pid 12345) source: driver(process)`, `Restart budget: 0 / 5 used`.
She knows immediately that the workload is up and the reconciler is
not retrying.

#### 2: Failed allocation post-broken-submit

After US-02's broken-binary submit failed, Ana runs `alloc status`.
Output shows `STATE: Failed`, `Last transition: Pending → Failed
reason: driver start failed source: driver(process) error: stat
/usr/local/bin/payments: no such file or directory`, `Restart
budget: 5 / 5 used (backoff exhausted)`. She knows the platform
gave up and what to fix.

#### 3: Pending — capacity exceeded

Ana submits a spec demanding 10 GiB on a 4 GiB host. Streaming
submit's terminal event is `ConvergedFailed { terminal_reason:
timeout, ... }` (because the lifecycle reconciler never schedules
it). She runs `alloc status` and sees an explicit `Pending: no node
has capacity (requested 10 GiB / free 3.2 GiB)` row, NOT a silent
zero-allocations render. She knows the platform did not "lose" the
spec.

### UAT Scenarios (BDD)

#### Scenario: alloc status renders a Running allocation with all snapshot fields

Given a job `payments-v2` has one allocation in `Running`
When Ana runs `overdrive alloc status --job payments-v2`
Then the output includes: state, resources, started timestamp, exit-code field
And the output includes a `Last transition:` block with from, to, reason, source, timestamp
And the output includes a `Restart budget:` line with used / max / exhausted

#### Scenario: alloc status renders a Failed allocation with the verbatim driver error

Given a job `payments-v2` has one allocation in `Failed` because the binary is missing
When Ana runs `overdrive alloc status --job payments-v2`
Then the output includes the verbatim driver error string
And the output includes `Restart budget: M / M used (backoff exhausted)` when the lifecycle reconciler has stopped retrying

#### Scenario: alloc status emits an honest empty-state for capacity-exceeded

Given a job whose resource request exceeds the local node capacity
When Ana runs `overdrive alloc status --job <id>`
Then the output includes a single explicit `Pending: no node has capacity (...)` row naming requested-vs-free
And the output does NOT silently render zero allocations

### Acceptance Criteria

- [ ] Snapshot wire shape carries: per-alloc `state`, `resources`,
  `started_at`, `exit_code`; top-level `last_transition` and
  `restart_budget`.
- [ ] CLI renders the TUI mockup from the journey YAML for Running
  and Failed cases.
- [ ] Verbatim driver error appears in the Failed case rendering.
- [ ] Restart budget appears with `(backoff exhausted)` annotation
  when applicable.
- [ ] Pending-with-no-capacity renders an explicit reason, not a
  silent zero.

### Outcome KPIs

- **Who**: Ana / second-day inspection operator.
- **Does what**: identifies allocation state, last transition,
  failure cause (if any), and reconciler retry posture from a
  single command.
- **By how much**: snapshot field count rises from 1 (`Allocations:
  N`) to ≥ 6 (state, resources, started, last-transition, error,
  restart-budget).
- **Measured by**: AC count of populated fields; visual inspection
  of the rendered output against the journey TUI mockup.
- **Baseline**: today's output is `Allocations: 1`. The journey
  extension already specified the target shape; this story makes
  it concrete.

### Technical Notes

- Trace: addresses ODI outcomes 3, 5 (time to identify reason;
  likelihood of re-deriving state from sparse output).
- Slice 1 ships this independently of streaming submit (no-regret).

---

## US-06 — Failure surfacing across submit and status (single source of truth)

### Problem

Two consumption surfaces — the streaming `ConvergedFailed` event and
the snapshot `last_transition.reason` — must show the same failure
reason for the same allocation. If the strings drift, the operator
gets two different diagnoses for one event and the platform's
"told the truth" promise is broken.

### Who

- Senior platform / SRE engineer | inspecting a Failed allocation
  via either surface | expects coherent, reproducible diagnosis.

### Solution

The `failure_reason` (and the underlying `transition_reason` for
each lifecycle transition) is sourced exactly once: from the
lifecycle reconciler view (for reconciler-domain reasons) or from
the ProcessDriver via the action shim (for driver-domain reasons).
Both the streaming event emitter and the snapshot hydrator read
from the same lineage. AC asserts byte-for-byte equality.

### Who reads what

- Streaming endpoint: emits `LifecycleTransition { reason }` and
  `ConvergedFailed { error }`.
- Snapshot handler: emits `last_transition.reason` and per-row
  `error` field.

### Domain Examples

#### 1: Same string in both surfaces — broken binary

After US-02, Ana captures the streaming submit's `ConvergedFailed`
event: `error: "stat /usr/local/bin/payments: no such file or
directory"`. She runs `alloc status` and the snapshot's
`last_transition` shows `error: stat /usr/local/bin/payments: no
such file or directory` — byte-identical.

#### 2: Same string — backoff exhaustion

The streaming `ConvergedFailed` carries `terminal_reason:
backoff_exhausted` and the same driver error string. The snapshot
shows `Restart budget: 5 / 5 used (backoff exhausted)` (a
human-readable rendering of the same machine-readable terminal
reason) AND the same driver error string in the
`last_transition.error` field.

#### 3: Same string — server timeout

Streaming `ConvergedFailed { terminal_reason: timeout, error: "did
not converge in 60s" }`. Snapshot shows `Last transition: Pending
→ ... reason: ... source: reconciler` reflecting the most recent
transition the reconciler observed; the snapshot does NOT display
the server-side timeout (the timeout is a streaming-specific
concern). Acceptable — the streaming surface and the snapshot
surface have different temporal scopes (point-in-time vs
event-stream); the SoT discipline applies to the
*per-transition* `reason` field, not to the streaming-only
terminal-cap concept.

### UAT Scenarios (BDD)

#### Scenario: streaming and snapshot agree on per-allocation transition reason

Given a streaming submit emitted a `LifecycleTransition` with reason R for allocation A
When Ana subsequently runs `overdrive alloc status --job <id>`
Then the snapshot's `last_transition.reason` for allocation A equals R verbatim

#### Scenario: streaming and snapshot agree on Failed driver error

Given a streaming submit emitted `ConvergedFailed { error: E }` for allocation A
When Ana subsequently runs `overdrive alloc status --job <id>`
Then the snapshot's per-row `error` field for allocation A equals E verbatim

### Acceptance Criteria

- [ ] One source of truth for `transition_reason` per allocation
  (lifecycle reconciler view + ProcessDriver pass-through).
- [ ] Streaming `LifecycleTransition.reason` ==
  snapshot `last_transition.reason` for the same allocation.
- [ ] Streaming `ConvergedFailed.error` == snapshot per-row
  `error` for the same allocation.
- [ ] An integration test asserts byte-for-byte equality across the
  two surfaces in the broken-binary regression case.

### Outcome KPIs

- **Who**: Ana, debugging via either surface.
- **Does what**: trusts that the two surfaces tell one story; never
  has to reconcile two different diagnoses.
- **By how much**: zero drift between surfaces. Boolean.
- **Measured by**: integration test in Slice 2 that submits a
  broken spec via streaming, captures the terminal event, runs
  `alloc status`, and asserts string equality on `reason` /
  `error` fields.
- **Baseline**: N/A (the streaming surface does not exist today).
  The discipline is established up front to prevent drift after
  Slice 2 lands.

### Technical Notes

- Trace: cross-cutting; protects the "told the truth" emotional
  promise across both surfaces. Cited in
  `shared-artifacts-registry.md` under `transition_reason`.
- Implementation: same `String` (or typed `Reason` enum if DESIGN
  prefers) flows through the lineage; no string formatting
  divergence between the two emit sites.
