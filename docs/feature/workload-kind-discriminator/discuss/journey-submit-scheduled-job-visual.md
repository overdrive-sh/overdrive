# Journey C — Submit a Scheduled Job

> **Workload kind**: `Job + Schedule` — recurring run-to-completion. Composes a Job's
> exit-code semantics with a Schedule's cron-driven fire policy. Equivalent to k8s CronJob,
> systemd `.timer`, Nomad periodic batch.
>
> **Persona**: Ana, Overdrive platform engineer.
>
> **Trace**: J-OPS-002 (primary).
>
> **Scope of THIS feature**: parser accepts `[job] + [schedule]` and validates composition
> rules. **Schedule execution semantics (cron parser, fire-on-tick, history retention) are
> a deferred follow-up — see `wave-decisions.md` § "Deferrals requiring user approval".**
> The CLI honestly reports that schedules parse but do not yet execute.

## Emotional arc

```
Curious → Confident-in-syntax → Patient
"how do I            "the parser     "the platform tells me
 declare a            accepted my     execution is not yet
 recurring            spec and        wired AND points me to
 job?"                acknowledged    the issue tracking it"
                      kind=Schedule"
```

The emotional cost of a deferred capability is *opacity*. The journey design counters that
with **explicit honesty**: the CLI says "Schedule registered — execution lands in #N" so
the operator knows where the road ends.

## ASCII flow

```
   STEP 1: Author Schedule spec   STEP 2: Submit          STEP 3: Honest deferral
   ---------------------------    --------------          ------------------------
   nightly-backup.toml            overdrive job          "Schedule 'nightly-backup'
                                  submit ./nightly-       registered.
   [job]                          backup.toml             cron: 0 2 * * *
   id = "nightly-backup"                                  next fire: (deferred)
   [schedule]                                             Execution support is not yet
   cron = "0 2 * * *"                                     implemented — see issue #166."
   [exec]
   command = "/usr/bin/..."        exit 0
                                   (parsed, validated,
   [resources]                     persisted as
   ...                             intent; no execution)

   FEELS: Curious                  FEELS: Confident-in-   FEELS: Patient (not Misled)
                                   syntax
```

## TUI mockups

### Step 1 — Schedule spec

```
$ cat ./nightly-backup.toml
+----------------------------------------------------------------+
| [job]                                                           |
| id = "nightly-backup"                                           |
|                                                                 |
| [schedule]                                                      |
| cron = "0 2 * * *"                                              |
|                                                                 |
| [exec]                                                          |
| command = "/usr/bin/pg_dump"                                    |
| args = ["--format=custom", "-d", "payments", "-f", "/backups"] |
|                                                                 |
| [resources]                                                     |
| cpu_milli = 200                                                 |
| memory_bytes = 134217728                                        |
+----------------------------------------------------------------+
```

### Step 2 — Submit acknowledgement

```
$ overdrive job submit ./nightly-backup.toml
+----------------------------------------------------------------+
| Submitting schedule 'nightly-backup' (kind=Schedule)           |
| Spec digest: sha256:c9e1...77                                   |
| Endpoint:    https://127.0.0.1:7001/                            |
| Schedule registered.                                            |
|                                                                 |
| NOTE: schedule execution is not yet implemented in this        |
|       Phase 1 slice. The spec has been validated and persisted |
|       as intent; no Job runs will be spawned automatically.    |
|       Tracking: github.com/overdrive-sh/overdrive/issues/166  |
+----------------------------------------------------------------+
```

CLI process exits with status `0` (the parse-and-validate happy path succeeded).

### Step 3 — alloc status reflects the deferral honestly

```
$ overdrive alloc status --job nightly-backup
+----------------------------------------------------------------+
| Job:    nightly-backup    (kind: Schedule)                     |
| Spec:   sha256:c9e1...77                                        |
| Cron:   0 2 * * *                                               |
|                                                                 |
| No allocations have been spawned yet.                          |
|                                                                 |
| Reason: Schedule execution is not yet implemented (issue #166).|
| The spec has been registered as intent; the Schedule           |
| reconciler that would fire Jobs on cron tick is the subject    |
| of a follow-up feature.                                        |
+----------------------------------------------------------------+
```

> The empty-allocation render explicitly names the deferral with a tracking URL —
> "honest about what it does and does not know" per J-OPS-002.

## Failure modes

- `[schedule]` with no `[job]` — parser rejects ("[schedule] is only valid alongside [job]").
- `[schedule]` AND `[service]` — parser rejects (services don't terminate; schedules need a
  bounded job per fire).
- `cron` value is malformed — parser rejects with the malformed substring named.
- Operator omits `cron` field inside `[schedule]` — parser rejects.

## Shared artifacts

- `${spec_path}` → operator filesystem.
- `${kind} = Schedule` → derived from `[job] + [schedule]` co-presence.
- `${cron_expr}` → string field inside `[schedule]`; surfaced unparsed in alloc status
  (parsing-as-cron-expression is part of the deferred execution work).
- `${deferral_issue_url}` → SSOT: a single config constant in the CLI naming the GH issue
  that tracks Schedule execution. Must be the SAME URL in submit echo AND alloc status
  output.

## Integration checkpoints

- The submit echo and the alloc status output reference the SAME deferral issue URL — the
  shared-artifacts registry pins this as a single source of truth.
- The CLI does NOT silently accept-and-discard the schedule. Persisting the spec as
  intent (via the existing IntentStore) preserves the Phase 1 walking-skeleton guarantee
  that "submitted things are committed."

## Cross-references

- `journey-submit-scheduled-job.yaml` — schema-form
- `journey-submit-scheduled-job.feature` — Gherkin scenarios
- `wave-decisions.md` § "Deferrals requiring user approval" — the deferral that gates the
  issue URL artifact
