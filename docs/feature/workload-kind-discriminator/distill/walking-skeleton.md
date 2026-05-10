# Walking Skeleton — workload-kind-discriminator

**Feature**: workload-kind-discriminator
**Wave**: DISTILL
**Strategy**: A (in-process direct-handler invocation; real local
adapters; `Sim*` traits for Clock/Transport/Entropy non-determinism
boundary). See `wave-decisions.md` § DWD-02 for rationale.

This file names the minimum end-to-end paths that prove the kind
discriminator wires together port-to-port. Each WS is demo-able to
a non-technical stakeholder (Mandate 5 / Dim-5 litmus test) and
exercises every driving port for its kind.

The four walking skeletons together cover all three workload kinds
(Service / Job / Schedule) plus the cross-kind alloc-status inspection
path (J-OPS-003).

---

## WS-01 — Service: Ana submits `payments` and sees a stable Service

**User goal**: "I want to declare a long-running service in TOML and
have the platform tell me, honestly, that it is up and serving
listeners I declared."

**Stakeholder demo**: 90 seconds. Open the TOML in an editor. Run
`overdrive job submit ./payments.toml`. CLI streams events in real
time, ending with `Service 'payments' is running with 1/1 replicas
(took 1.4s)`. Run `overdrive alloc status --job payments`. CLI prints
the kind-aware Service render: header (kind: Service), Replicas
1/1, Listeners section with two triples, per-alloc table without an
Exit column.

**Driving ports traversed** (each invoked by its actual entry-point shape):

1. **TOML parser** — `WorkloadSpecInput::deserialize(toml_bytes)`
   sees `[service]` + two `[[listener]]` blocks, branches to
   `Service` variant.
2. **`overdrive job submit` CLI handler** —
   `commands::job::submit(SubmitArgs { spec, config_path }, &Clock,
   &Transport)` returns `SubmitOutput { spec_digest, endpoint, ... }`.
3. **IntentStore write** — `IntentStore::put_if_absent(IntentKey,
   WorkloadSpec::Service(...))` against real `redb` in
   `tempfile::TempDir`.
4. **Streaming subscriber** — `streaming::dispatcher` selects
   `ServiceSubmitEvent` per intent kind; observation-store
   subscription emits `ConvergedRunning` on first stable Running row.
5. **CLI render dispatcher** — `render::dispatch` routes
   `ServiceSubmitEvent::ConvergedRunning` to `render::service::format_running_summary`,
   which produces "Service 'payments' is running with 1/1 replicas
   (took 1.4s)".
6. **`overdrive alloc status` CLI handler** —
   `commands::alloc_status::status(StatusArgs { job, config_path },
   ...)` returns `AllocStatusOutput`.
7. **ObservationStore read** — real `redb` read of
   `AllocStatusRow { kind: Service, listeners: Vec<ListenerRow>, ... }`.
8. **Render dispatcher (status branch)** — kind-aware Service render;
   includes Listeners section with byte-equal triples.

**Observable user outcomes** (Then steps in user terms):

- CLI streaming output names the workload as a Service.
- Duration is a measured value (e.g. "1.4s"), NOT the literal `"live"`.
- `alloc status` shows the same listener triples Ana declared, in
  declaration order, with their `(vip, port, protocol)` rendered the
  same way the submit echo printed them (K6 byte-equality).
- The per-alloc table has columns `Alloc / State / Restarts / Since`
  and NO Exit column.

**Real-I/O scope**: real `tempfile::TempDir` redb files for IntentStore
+ ObservationStore; real `toml::de` parse; real NDJSON streaming wire
format. `SimDriver` for the workload exec (Service path; we don't
need real cgroup writes for this WS — the K1 honesty test in WS-02
covers real cgroup). `SimClock` for measured-duration determinism.

**Stories covered**: US-01 (parser), US-04 (Service preservation;
"live" literal removed), US-08 (listener spec shape).

---

## WS-02 — Job: Ana submits `coinflip` and gets a definitive verdict

**User goal**: "I want to run a one-shot script and have the CLI tell
me — honestly — whether it succeeded or failed, with the actual exit
code, never a fabricated 'running' line."

**Stakeholder demo**: 60 seconds. Show the bug under audit reproduced
on the OLD shape (CLI prints `is running with 1/1 replicas (took
live)` for an exit-1 workload). Migrate `examples/coinflip.toml` to
the new `[job]` shape. Re-run `overdrive job submit
examples/coinflip.toml` with the failing branch. CLI streams
`attempt 1 failed (exit 1, 0.2s). Retrying in 0.5s...`, then attempt
2, then attempt 3, then `Job 'coinflip' failed.\n  exit code: 1\n
duration: 0.3s (per-attempt)\n  attempts: 3 of 3 (backoff exhausted)\n
stderr (last 5 lines):\n  ERROR`. CLI process exits with status 1.
Run `overdrive alloc status --job coinflip` and see the kind-aware
Job render: Verdict line, per-attempt table with Exit column, stderr
tail.

**Driving ports traversed**:

1. **TOML parser** — `WorkloadSpecInput::deserialize(toml_bytes)`
   sees `[job]` only, branches to `Job` variant.
2. **`overdrive job submit` CLI handler** — same shape as WS-01 but
   the streaming subscriber follows the Job code path.
3. **IntentStore write** — same shape as WS-01.
4. **JobLifecycle reconciler tick** — emits typed
   `TerminalCondition::Failed { exit_code: 1 }` after the third
   attempt's exit observation, per ADR-0037 Amendment 2026-05-10.
5. **Streaming subscriber (Job sub-path)** — waits for ExitObserver's
   terminal observation row; emits `JobSubmitEvent::AttemptFailed`
   intermediate events, then `JobSubmitEvent::Failed` terminal.
6. **CLI render dispatcher** — routes `JobSubmitEvent::Failed` to
   `render::job::format_failed_summary`. NEVER reaches
   `render::service::format_running_summary` (compile-time
   exhaustive match on per-kind enum).
7. **CLI process-exit boundary** — `commands::job::submit` returns
   non-zero `process_exit_code` from the terminal verdict; binary
   wrapper maps to `std::process::exit(1)`.
8. **`overdrive alloc status` CLI handler** — reads `AllocStatusRow`
   with `kind: Job` and per-attempt rows; render branch produces
   Job-shaped output.

**Observable user outcomes**:

- CLI's verdict line for the failing run says "failed", not "running".
- The exit code rendered (`1`) equals the kernel-observed exit code
  recorded by the worker's ExitObserver.
- The CLI process exit equals the workload exit (status 1).
- `alloc status` shows three rows (one per attempt) each with Exit
  "1" and the captured stderr tail.
- No line of any output (streaming OR `alloc status`) contains the
  substring `is running with` (anti-scenario).

**Real-I/O scope** (full Tier 3 for K1 honesty): real `ExecDriver`
+ real `/bin/bash` subprocess + real cgroup writes under
`/sys/fs/cgroup/overdrive.slice/workloads.slice/`; real `redb` for
both stores; real ExitObserver pipeline. K1 (≥99% honesty over 100
trials) is the gate; routed through Lima per
`.claude/rules/testing.md` § "Running tests — Lima VM".

For non-K1 default-lane tests (the parser-and-render scenarios for
this WS), `SimDriver` configured to inject a scripted exit-1
sequence is sufficient. The crafter writes both: K1 as a single
gated Tier-3 test, the per-step assertions as default-lane tests.

**Stories covered**: US-01 (parser), US-02 (Job submit terminal),
US-07 (coinflip migration). Anti-scenarios (no `is running with`,
no `(took live)`) covered as part of US-02.

---

## WS-03 — Schedule: Ana registers `nightly-backup` and gets honest deferral

**User goal**: "I want to declare a recurring backup in TOML, have
the platform validate the spec today, and tell me — honestly — when
execution support arrives. I want to commit my Schedule manifest now
without the platform pretending to do work it can't."

**Stakeholder demo**: 45 seconds. Open `nightly-backup.toml` showing
`[job]` + `[schedule]` with `cron = "0 2 * * *"`. Run `overdrive job
submit ./nightly-backup.toml`. CLI prints `Submitting schedule
'nightly-backup' (kind=Schedule)`, the spec digest, the endpoint,
then `Schedule registered.\n\nNOTE: schedule execution is not yet
implemented in this Phase 1 slice…\n      Tracking:
https://github.com/overdrive-sh/overdrive/issues/166`. CLI exits 0.
Run `overdrive alloc status --job nightly-backup`. CLI prints the
kind-aware Schedule render: kind, spec digest, cron expression, "No
allocations have been spawned yet", and a Reason line referencing
the same #166 URL byte-for-byte.

**Driving ports traversed**:

1. **TOML parser** — `WorkloadSpecInput::deserialize(toml_bytes)`
   sees `[job]` AND `[schedule]`, branches to `Schedule` variant.
2. **`overdrive job submit` CLI handler** — same shape as WS-01/02;
   streaming subscriber follows Schedule code path.
3. **IntentStore write** — `WorkloadSpec::Schedule` persisted (per
   J-OPS-002 — submitted things are committed even if execution is
   deferred).
4. **Streaming subscriber (Schedule sub-path)** — emits `Accepted` +
   `Registered { cron, deferral_url }` immediately at submit; stream
   closes (no firing semantics this slice).
5. **CLI render dispatcher** — routes to
   `render::schedule::format_registered`, which reads
   `SCHEDULE_EXECUTION_TRACKING_URL` constant (single SSOT per
   ADR-0047 §1).
6. **`overdrive alloc status` CLI handler** — reads
   `AllocStatusRow` with `kind: Schedule`; render branch reads the
   same constant.

**Observable user outcomes**:

- CLI prints "Schedule registered." not "Job is running…".
- The deferral URL printed at submit time and the deferral URL printed
  at `alloc status` time are byte-identical (K5 KPI).
- CLI exits 0 (Schedule submit is accepted; execution deferral is
  not a failure).
- The cron string Ana wrote is echoed back unchanged in `alloc status`.

**Real-I/O scope**: real `tempfile::TempDir` redb; real TOML parse;
real NDJSON streaming. No driver involved (Schedule kind doesn't
spawn allocations this slice).

**Stories covered**: US-01 (parser supports Schedule kind), US-05
(Schedule parsing + honest deferral).

---

## WS-04 — Cross-kind: Ana inspects all three live workloads via `alloc status`

**User goal**: "I want to use one command — `overdrive alloc status
--job <id>` — to inspect any workload in the cluster, and have it
tell me what I need to know in terms that match the workload's kind.
For a Service: replicas + uptime. For a Job: verdict + per-attempt
exit codes. For a Schedule: cron + deferral notice. The same command,
three coherent outputs."

**Stakeholder demo**: 90 seconds. Three workloads from WS-01, WS-02,
WS-03 are live. Run `overdrive alloc status --job payments`,
`overdrive alloc status --job coinflip`, `overdrive alloc status
--job nightly-backup` in sequence. Each output speaks the right
vocabulary for its kind. No output contains cross-kind phrasing
(no "is running with" for Job; no "Verdict" for Service; no per-alloc
table for Schedule).

**Driving ports traversed** (third invocation of the alloc status
boundary; each kind picks its own render branch from the same
dispatcher):

1. **`overdrive alloc status` CLI handler** — invoked three times,
   each time with a different `job_id`.
2. **ObservationStore read** — three reads of `AllocStatusRow`, each
   carrying its kind tag + kind-specific fields.
3. **Render dispatcher** — `match alloc_status_row.kind { Service =>
   render::service::status, Job => render::job::status, Schedule =>
   render::schedule::status }`. Exhaustive — adding a future kind
   means adding one match arm.

**Observable user outcomes**:

- Service render: `kind: Service`, `Replicas (desired/running)`,
  `Listeners:` section, Alloc/State/Restarts/Since table, NO Exit
  column.
- Job render: `kind: Job`, `Verdict: {Succeeded, Failed (backoff
  exhausted), In progress}`, Attempt/State/Exit/Started/Duration
  table, stderr tail on Failed.
- Schedule render: `kind: Schedule`, `Cron: <expr>`, "No allocations
  have been spawned yet", `Reason: Schedule execution is not yet
  implemented (issue #166).`
- For the Job render specifically: no line contains the substring
  `is running with` (anti-scenario, structural — covered by Mandate 1
  invariant on `JobSubmitEvent` enum, observable via render output).

**Real-I/O scope**: real `redb` ObservationStore; real `commands::alloc_status::status`
direct calls; in-memory render assertions (string contains / does
not contain).

**Stories covered**: US-03 (alloc status kind-aware Job render),
US-04 (Service preservation), US-05 (Schedule deferral),
US-08 (Service Listeners section in alloc status).

---

## What is intentionally NOT in any walking skeleton

- **The runtime VIP allocator behaviour for `vip = None`** —
  tracked at GH #167. Slice 06 ships the spec field shape only; the
  runtime decision is a separate primitive. WS-01's listener with
  `vip = None` renders as `(vip: pending allocation — see #167)` —
  the WS does NOT exercise allocator code (there is none yet).
- **Schedule execution semantics** — tracked at GH #166. WS-03 stops
  at "Registered + deferral notice"; the WS does NOT exercise
  cron firing, ConcurrencyPolicy, or history retention.
- **Service settle window / health-check primitive** — tracked at
  GH #170. WS-01 uses an in-test stable workload (no startup
  flakiness); the WS does NOT exercise the RCA root cause A
  surface. (A future WS for #170 will.)
- **Real cross-region / multi-node** — Phase 1 is single-binary;
  the WS deliberately runs in-process to mirror production shape.

These exclusions are deliberate per `wave-decisions.md` § DWD-12.

---

## Adapter coverage summary

| Driven adapter | Real-I/O test | WS that traverses it |
|---|---|---|
| TOML deserialiser | `parser_accepts_*` family in §1 | WS-01, WS-02, WS-03 |
| `redb` IntentStore | `submit_persists_intent_round_trip` | WS-01, WS-02, WS-03 |
| `redb` ObservationStore | `alloc_status_reads_persisted_row` | WS-01, WS-02, WS-03, WS-04 |
| `ExecDriver` (real cgroup) | `coinflip_honesty_100_trials` (K1 — Tier 3, Lima) | WS-02 |
| OpenAPI generator | `openapi_schema_includes_listener` + `cargo openapi-check` | WS-01 (transitively — Listener type) |
| `xtask::dst_lint` `"live"` rule | `dst_lint_rejects_live_literal` | not part of any WS — covered as a §6 focused scenario |
| Streaming NDJSON wire | existing `streaming_submit_happy_path` extended | WS-01, WS-02, WS-03 |

Every adapter has at least one real-I/O test (Dim-9c green). No WS
relies on `@in-memory` doubles for a local resource adapter — Strategy
A specifies real local adapters at every named boundary.

---

## Why these four, not more

`nw-test-design-mandates` § "Walking Skeleton Strategy" prescribes
2-5 per feature. Four matches the four bounded user goals
(submit Service, submit Job, submit Schedule, inspect all three).
Adding a fifth WS for "submit a Service that fails to stabilise"
would test the RCA root cause A surface — which is explicitly
out of scope (GH #170). Adding a fifth WS for "submit two listeners
sharing a `(port, protocol)` triple" would be a focused error-path
scenario, not a WS — it does not deliver observable user value.

The four are minimum sufficient to demo "the kind discriminator
works end-to-end for every kind, and inspection works honestly for
each."
