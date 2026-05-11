# Journey D — Inspect alloc status (cross-kind, kind-aware semantics)

> **Changed Assumptions** — 2026-05-10: folded in GH #164 (service listener
> spec shape). The Service render branch (Sub-path D1) now includes a
> `Listeners:` section with one line per declared listener. Pending VIPs
> reference [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).
> The Job and Schedule sub-paths are unchanged.

> **Cross-cutting**: this journey is about a single CLI command (`overdrive alloc status
> --job <id>`) whose render branches on the workload kind. It is the operator's primary
> "what really happened?" surface — and the user explicitly framed this journey when they
> said:
>
> > *"in the terms of a job, it is actually correct behavior. what should happen, is when
> > we check the status using overdrive alloc status --job <id> then it should show that
> > it failed during execution"*
>
> **Persona**: Ana, Overdrive platform engineer.
>
> **Trace**: J-OPS-003 (primary — alloc status is the whole job statement), J-OPS-002
> (secondary).

## Emotional arc

```
Curious / Wondering   →   Reading carefully   →   Confident
"what's the actual         "the kind label         "the platform's view
 state right now?"          tells me which          matches my mental
                            semantics apply"        model for THIS kind"
```

For a Service operator: confidence comes from "Running" being honest steady-state.
For a Job operator: confidence comes from seeing the **terminal verdict** (`Succeeded` /
`Failed`) with the actual `exit_code` — not "Running" left dangling from the streaming
window.

## ASCII flow — three sub-paths by kind

```
   STEP 1: Operator runs       STEP 2: CLI reads kind from   STEP 3: Render kind-aware
   `overdrive alloc status     AllocStatusRow.kind            view
    --job <id>`
                               kind = Service →               Replicas (desired/running)
                                                              Per-alloc table with
                                                              State / Restarts / Since

                               kind = Job →                   Terminal verdict line
                                                              (Succeeded / Failed / Running)
                                                              Per-attempt history with
                                                              exit_code per attempt

                               kind = Schedule →              Cron + last-run-result
                                                              Or deferral notice (this slice)

   FEELS: Curious              FEELS: Reading carefully       FEELS: Confident
```

## TUI mockups

### Sub-path D1 — Service status (with listeners)

```
$ overdrive alloc status --job payments
+----------------------------------------------------------------+
| Job:    payments    (kind: Service)                            |
| Spec:   sha256:a4c1...e9                                        |
| Replicas (desired/running): 1/1                                |
| Listeners:                                                      |
|   10.0.0.1:8080/tcp                                             |
|   (vip: pending allocation - see #167):8081/udp                 |
|                                                                 |
| Alloc                  State    Restarts  Since                |
| ---------------------- -------- --------- ----------           |
| payments-0             Running  0         00:00:42.1            |
+----------------------------------------------------------------+
```

> The `Listeners:` block byte-equals the section printed in the submit echo
> for the same spec. Pinned VIPs render the IPv4 literal; absent VIPs render
> `(vip: pending allocation - see #167)` referencing the runtime allocator
> tracked at
> [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).
> Round-trip integrity is asserted by KPI K6.

### Sub-path D2 — Job status (Succeeded — terminal)

```
$ overdrive alloc status --job coinflip
+----------------------------------------------------------------+
| Job:      coinflip    (kind: Job)                              |
| Spec:     sha256:b7f2...3a                                      |
| Verdict:  Succeeded                                            |
|                                                                 |
| Attempt  State       Exit  Started               Duration      |
| -------  ----------  ----  --------------------  --------       |
| 1        Succeeded   0     2026-05-09T14:27:02Z  1.2s            |
+----------------------------------------------------------------+
```

### Sub-path D2' — Job status (Failed — terminal, exit code visible)

```
$ overdrive alloc status --job coinflip
+----------------------------------------------------------------+
| Job:      coinflip    (kind: Job)                              |
| Spec:     sha256:b7f2...3a                                      |
| Verdict:  Failed (backoff exhausted)                           |
|                                                                 |
| Attempt  State       Exit  Started               Duration      |
| -------  ----------  ----  --------------------  --------       |
| 1        Failed      1     2026-05-09T14:27:02Z  0.2s            |
| 2        Failed      1     2026-05-09T14:27:03Z  0.2s            |
| 3        Failed      1     2026-05-09T14:27:05Z  0.3s            |
|                                                                 |
| Last stderr (alloc coinflip-3, last 3 lines):                  |
|   ERROR                                                         |
+----------------------------------------------------------------+
```

> **This is the journey the user explicitly named.** When the bug-affected coinflip
> workload exits 1, the operator running `alloc status` sees `Verdict: Failed`,
> `Exit: 1` per attempt, and the actual stderr — not "Running".

### Sub-path D2'' — Job status (Running — mid-flight, not yet terminal)

```
$ overdrive alloc status --job long-import
+----------------------------------------------------------------+
| Job:      long-import    (kind: Job)                           |
| Spec:     sha256:e2a3...11                                      |
| Verdict:  In progress (no terminal yet)                        |
|                                                                 |
| Attempt  State       Exit  Started               Duration      |
| -------  ----------  ----  --------------------  --------       |
| 1        Running     —     2026-05-09T14:27:02Z  00:02:13       |
+----------------------------------------------------------------+
```

> "In progress" is the kind-aware vocabulary for "the Job is currently executing but has
> not yet exited." The rendered `Exit` column is em-dash (not "0", not blank) until the
> attempt terminates — empty states get explicit visual content per the UX
> emotional-design skill.

### Sub-path D3 — Schedule status (deferred execution; same as journey C step 3)

See `journey-submit-scheduled-job-visual.md` — the alloc status render for a Schedule with
no spawned jobs honestly says "execution is not yet implemented (issue #166)".

## Failure modes

- `--job <id>` names a job that does not exist — typed not-found error.
- The persisted `AllocStatusRow.kind` is missing (e.g. a row written by a pre-feature
  control plane) — the CLI must NOT panic; render with "kind: Unknown" and a hint that
  the row predates the kind discriminator.
- Job's `terminal_exit_code` field is missing on a row whose state is `Failed` (data-shape
  bug) — render "Exit: ?" rather than crashing.

## Shared artifacts

- `${alloc_status_kind}` — `AllocStatusRow.kind`; denormalised at write time from the
  spec's declared kind. Single source of truth.
- `${exit_code}` — per-attempt; sourced from the existing ExitObserver Phase 1 path.
- `${attempt_count}` — derived from the rows' restart-attempt history, not a separate
  counter.
- `${spec_digest}` — `ContentHash::of(rkyv archive)`; identical to the value the submit
  echo printed.
- `${duration}` — Clock-derived; never literal strings.
- `${listener_triple}` — `Vec<Listener>` for Service kind; denormalised on
  `AllocStatusRow` at write time (architect to confirm shape). Round-trip
  byte-identical with the submit echo Listeners section.
- `${vip_assignment_state}` — `Option<ServiceVip>` per listener. Pinned IPv4
  vs. `(vip: pending allocation - see #167)` literal.

## Integration checkpoints

- The **kind label** in alloc status MUST equal the kind that was declared in the
  originally-submitted spec. Mismatch is a protocol violation — the test suite asserts on
  this.
- For a Job, the `Verdict` line is one of `{Succeeded, Failed, Failed (backoff exhausted),
  In progress}` — never the substring "Running with N/M replicas". That phrasing is
  reserved for Service kind.
- For a Service, the per-alloc table never carries an `Exit` column — exit codes are not
  the operator-relevant signal; restart count and uptime are.
- For a Service with M listeners, the `alloc status` Listeners section has exactly M
  lines, in declaration order, each line byte-equal to the corresponding submit echo
  line for the same spec. KPI K6 asserts this byte-equality across 100 trials with
  pinned VIPs.

## Cross-references

- `journey-alloc-status.yaml` — schema-form
- `journey-alloc-status.feature` — Gherkin scenarios
- `shared-artifacts-registry.md` — `alloc_status_kind` is the highest-risk artifact
