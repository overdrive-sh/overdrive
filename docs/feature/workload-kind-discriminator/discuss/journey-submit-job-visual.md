# Journey B — Submit a Job (run-to-completion)

> **Workload kind**: `Job` — run-to-completion. Success = exit 0; failure = exit non-zero.
> Equivalent to k8s Job, Nomad `batch`, Cloud Run job.
>
> **Persona**: Ana, Overdrive platform engineer.
>
> **Trace**: J-OPS-002 (primary).
>
> **This is the journey the bug exposed.** The current Phase 1 shape returns
> "running with 1/1 replicas" for a Job that exits with status 1. After this feature
> lands, that line is structurally unreachable for a Job; the streaming protocol terminates
> on `Succeeded { exit_code: 0 }` or `Failed { exit_code: N != 0 }`.

## Emotional arc — confidence built on honesty

```
Anxious → Watchful → Either:
  "did                "the job is             "Succeeded.
   it run?"            running, but            exit 0 in 1.2s"     → Satisfied
                       I haven't seen     OR
                       the verdict yet"
                                            "Failed.
                                             exit 1 in 0.3s.
                                             Restart 1 of 3 in   → Informed-but-not-misled
                                             0.5s..."
                                                                    (NOT "running")
```

The bug's emotional cost was *false confidence*: the operator was told "running" and
believed the workload had succeeded, then saw the ERROR log and lost trust in the CLI.
This journey replaces false confidence with **honest uncertainty during the run, then a
definitive verdict at the end**.

## ASCII flow

```
   STEP 1: Author Job spec    STEP 2: Submit          STEP 3: Stream to verdict
   -------------------------   ----------------------  --------------------------
   coinflip.toml               overdrive job          → Accepted
                               submit ./coinflip.toml  → Pending
   [job]                                               → Running (transient!)
   id = "coinflip"                                     → Either:
                                                         Succeeded { exit_code: 0,
   [exec]                                                            duration: 1.2s }
   command = "/bin/bash"                                OR:
   args = ["-c", "..."]                                  Failed { exit_code: 1,
                                                                  duration: 0.3s,
   [resources]                                                    will_restart: true,
   cpu_milli = 100                                                attempt: 1/3 }
   memory_bytes = ...
                                                       → After backoff exhausted:
                                                         Failed-Final { exit_code: 1 }

   FEELS: Anxious              FEELS: Watchful         FEELS: Satisfied (Succeeded)
                                                       OR: Informed (Failed-Final)
                                                       NEVER: Misled
```

## TUI mockups

### Step 1 — Job spec

```
$ cat ./coinflip.toml
+----------------------------------------------------------------+
| [job]                                                           |
| id = "coinflip"                                                 |
|                                                                 |
| [exec]                                                          |
| command = "/bin/bash"                                           |
| args = [                                                        |
|   "-c",                                                         |
|   "if (( RANDOM % 2 )); then echo SUCCESS; exit 0; else \\      |
|    echo ERROR >&2; exit 1; fi"                                  |
| ]                                                               |
|                                                                 |
| [resources]                                                     |
| cpu_milli = 100                                                 |
| memory_bytes = 67108864                                         |
+----------------------------------------------------------------+
```

### Step 2 — Submit echo

```
$ overdrive job submit ./coinflip.toml
+----------------------------------------------------------------+
| Submitting job 'coinflip' (kind=Job, run-to-completion)        |
| Spec digest: sha256:b7f2...3a                                   |
| Endpoint:    https://127.0.0.1:7001/                            |
| Waiting for terminal exit (Succeeded or Failed)...             |
+----------------------------------------------------------------+
```

### Step 3 — Streaming converges Succeeded (exit 0)

```
+----------------------------------------------------------------+
| Job 'coinflip' succeeded.                                      |
|                                                                 |
|   exit code: 0                                                 |
|   duration:  1.2s                                              |
|   attempts:  1                                                  |
|                                                                 |
|   Run `overdrive alloc status --job coinflip` for full state. |
+----------------------------------------------------------------+
```

CLI process exits with status `0`.

### Step 3' — Streaming converges Failed (non-zero exit, no more restarts)

```
+----------------------------------------------------------------+
| Job 'coinflip' failed.                                         |
|                                                                 |
|   exit code: 1                                                 |
|   duration:  0.3s (per-attempt)                                |
|   attempts:  3 of 3 (backoff exhausted)                        |
|                                                                 |
|   stderr (last 5 lines):                                       |
|     ERROR                                                       |
|                                                                 |
|   Run `overdrive alloc status --job coinflip` for full state. |
+----------------------------------------------------------------+
```

CLI process exits with status `1` (non-zero — operator-readable failure signal).

### Step 3'' — Streaming intermediate event (Failed but will retry)

```
+----------------------------------------------------------------+
| Job 'coinflip' attempt 1 failed (exit 1, 0.2s). Retrying       |
| in 0.5s... (attempt 2/3)                                       |
+----------------------------------------------------------------+
```

> Each retry produces one of these intermediate lines; the streaming session terminates
> only on the final Succeeded or Failed event.

## Failure modes

- Spec carries `[job]` AND `[service]` — parser rejects.
- Workload exits with non-zero code on every attempt → eventually `Failed-Final`.
- Workload exits 0 on attempt N → `Succeeded` (success on any attempt).
- Streaming cap (60s default) elapses before terminal exit → emit kind-aware Timeout
  event ("Job 'coinflip' did not reach a terminal state within 60s; check `alloc status`").

## Shared artifacts

- `${spec_path}` → operator filesystem.
- `${kind} = Job` → derived from `[job]` section presence.
- `${exit_code}` → from `ExitObserver` (already exists in Phase 1); now flows through to
  the streaming protocol's terminal event.
- `${duration}` → from injected `Clock`; never a hard-coded literal.

## Integration checkpoints

- **The bug's structural fix lives here.** A Job-kind submit's stream events are typed
  `JobSubmitEvent`, which has variants `{ Accepted, Pending, Running, AttemptFailed,
  Succeeded, Failed }`. There is **no** `ConvergedRunning` variant — the call site that
  emits "is running with N/M replicas (took live)" does not exist for this code path.
- The CLI exit code mirrors the workload's terminal status: 0 on Succeeded, 1 on Failed.
- The CLI vocabulary is "Job" (capital-J kind), not the lowercase "job" used as the CLI
  noun in `overdrive job submit`. (`overdrive job submit` is the verb against the spec
  file regardless of kind; the parsed kind drives the streaming and render branches.)

## Cross-references

- `journey-submit-job.yaml` — schema-form
- `journey-submit-job.feature` — Gherkin scenarios
- `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md` — the
  bug this journey closes structurally
