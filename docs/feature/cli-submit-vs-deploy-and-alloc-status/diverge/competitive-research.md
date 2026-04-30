# Competitive Research — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DIVERGE / Phase 2
**Owner**: Flux (`nw-diverger`)
**Date**: 2026-04-30
**Research depth**: Lightweight — 3+ named competitors, market priors
already strong (per invocation).

---

## Scope

The user explicitly named the comparable products. This document
summarises each via prior knowledge of their command surface; no web
fetches were performed (none were needed, and the user is a senior
platform/SRE engineer who recognises every shape below). The goal is
to ground the six structural options (see `options-raw.md`) in real
recognisable shapes, not to discover new competitors.

---

## 1. `kubectl apply` + `kubectl rollout status`

### Submit shape

`kubectl apply -f deployment.yaml` is **fire-and-forget**. Returns
immediately on validation success (`deployment.apps/payments
configured`). Reconciliation happens asynchronously in the controller
manager; the apply call returns before any pod has been scheduled.

### Wait shape

`kubectl rollout status deployment/payments` is the explicit-wait
companion. Streams progress; exits 0 when desired replicas == ready
replicas; exits non-zero on rollout failure or `--timeout` expiry.

`kubectl wait --for=condition=available deployment/payments` is the
generic flag-driven companion (works on any condition).

### Status shape — multi-command

Four separate commands in the diagnostic toolkit:

- `kubectl get pods` — pod-level summary (`STATUS`, `RESTARTS`, age).
- `kubectl describe pod <name>` — verbose; the failure reason
  (`ImagePullBackOff`, `CrashLoopBackOff`) lives here.
- `kubectl logs <pod>` — workload stdout/stderr.
- `kubectl get events --field-selector
  involvedObject.name=<name>` — chronology.

None alone gives the whole story.

### What it does well

- Verb decoupling is explicit. Operator opts into waiting via a
  separate verb.
- CI scripts choose blocking or non-blocking trivially; mode is
  expressed at invocation time, not bound to the apply call.
- `wait --for=condition=...` is genuinely composable — works with
  any custom resource without per-resource wait verbs.

### Where it fails this job

- **Diagnostic surface is fragmented** across four commands; the
  operator must know which one provides which piece. This *directly*
  fails outcome 3 (time to identify convergence failure reason)
  unless the operator already has the full toolkit memorised.
- The two-verb shape is *correct* as a structural split but the
  status surface fails the inner-loop test: it minimises latency at
  the cost of cognition load.

### Key assumption

Operators are willing to compose multiple commands and know which
one provides which piece. Reasonable for senior SREs; awful for
first-day-with-the-platform.

---

## 2. `nomad job run`

### Submit shape — the closest sibling to Overdrive's current state

`nomad job run payments.nomad` is the closest reference for what
Overdrive is shipping today — except Nomad **waits by default**. It
streams the evaluation state and exits when the deployment is healthy
(or fails). `--detach` flips it to fire-and-forget, returning the
evaluation ID immediately.

### Wait shape

Built into `run` by default. `nomad deployment status <id>` is the
explicit out-of-band check (used after a `--detach`).

### Status shape — `nomad alloc status`

This is the canonical command and **is the shape Overdrive's current
`alloc status` should aspire to**. One command, dense output:

- Allocation state (`running` / `pending` / `failed` / `complete` /
  `lost`).
- **Task events** — chronological log of every state transition with
  category and human reason. The diagnostic gold:

  ```
  Recent Events:
    Time                 Type         Description
    2026-04-29T10:14:32  Task Setup   Building Task Directory
    2026-04-29T10:14:32  Driver       Failed to start: stat /usr/bin/nonexistent: no such file or directory
    2026-04-29T10:14:32  Driver Failure   Failed to start due to error: ...
  ```

- Resource utilization (CPU, memory).
- Recent log tail.
- Restart history.

### What it does well

- Default-wait means a broken submit returns non-zero with a reason
  in the same terminal where you typed the command. The exit code
  carries diagnostic signal.
- `alloc status`'s task-events panel is the single most successful
  CLI diagnostic format among the four references — every transition
  has a timestamp, category, and human-readable reason.

### Where it fails this job

- `--detach` is non-default but is what every CI script uses,
  leading to a two-personality CLI. Operators who learn the
  interactive default get burned when their CI pipeline behaves
  differently.
- Status output is dense to the point of overwhelming for the happy
  path — operator scans a lot to find the one line they care about.

### Key assumption

Operators are okay with waiting on the command line by default.
Holds for interactive use; can break in CI / scripting / long-running
jobs unless `--detach` is muscle-memory.

---

## 3. `fly deploy`

### Submit shape — single-verb, default-wait

`fly deploy` is the canonical verb. **Waits by default** with a
multi-strategy progress display:

```
==> Building image
==> Pushing image to registry
==> Deploying machines
==> Health-checking machines
==> Marking ready
```

On failure, prints the failing machine's logs inline.

### Wait shape

Built into `deploy`. **No separate wait verb** — the wait IS the
deploy.

### Status shape

- `fly status` — live machine table with states.
- `fly logs` — streams logs.
- `fly checks list` — health-check state.

The deploy command itself is the **primary diagnostic surface** —
failures during deploy print the relevant logs without the operator
typing a follow-up command.

### What it does well

- One verb, one experience. The "what happened" is co-located with
  the "I asked for it to happen" command.
- Pattern is essentially "the verb is the diagnostic." For
  interactive use this is the best operator experience among the
  four references by a clear margin.

### Where it fails this job

- Deploy is **monolithic** — there's no clean split between "send
  the spec" and "wait for it to converge." For automation that wants
  pipelined apply-then-check-elsewhere, the operator has to fight
  the default.
- Bundles building/pushing/deploying — too much for a control-plane-
  only orchestrator like Overdrive (no image-build phase to bundle).

### Key assumption

The interactive case is the dominant case, and CI is the exception.
Holds for fly's actual user base; less obviously true for a
control-plane platform that will be consumed from `cargo xtask` and
GitHub Actions as well as interactively.

---

## Reference points (out-of-tree, useful priors)

### `docker run`

Foreground by default; `-d` to detach. Failure to start IS the command
failure (non-zero exit, error on stderr). Pure synchronous shape.
Status observed via the running command's stdout/stderr; `docker ps`,
`docker logs`, `docker inspect` are out-of-band. The "PID 1 IS the
command" model — works when the workload is the foreground process,
breaks when the workload is a control-plane resource.

### `systemctl start <unit>` + `journalctl -fu <unit>`

`systemctl start` returns when activate phase exits — semantics
governed by the unit's `Type=`:

- `Type=simple` — returns instantly; not particularly useful for
  feedback.
- `Type=notify` — waits for `READY=1` from the unit; honest "ready"
  signal.
- `Type=forking` — waits for the parent fork to exit.

Failure to activate surfaces inline. **The diagnostic pattern senior
SREs actually use is `systemctl start && journalctl -fu`** — fire the
command, watch the log stream that was already open in another
terminal. This decouples *trigger* from *observation* without forcing
the trigger to wait, and the log stream is honest about what the unit
is doing in real time. This is the **non-obvious alternative** the
options canvas should reflect.

`systemctl status <unit>` is a separate snapshot surface — shows
state, recent log lines, and last-failure reason in one screen. Dense
single-screen render; the closest reference for a "rich snapshot
status" pattern (Option M in the options canvas).

### `terraform plan` + `terraform apply`

Two-verb declarative flow with a confirmation gate. `plan` shows
what *would* happen; `apply` actually mutates state. `-auto-approve`
skips the confirm. Each resource's create/update/delete is logged
inline. The plan/apply split is the precedent for Option R.

Different mental model from kubectl/nomad/fly — terraform users
expect to see the diff before committing; ops users expect to commit
and observe. Neither is wrong; they fit different problem shapes.

---

## Cross-cutting patterns table

| Pattern | Examples | Cost |
|---|---|---|
| Default-async, opt-in wait via flag | `kubectl apply` + `kubectl rollout status` / `kubectl wait` | Two mental modes; operator decides per invocation |
| Default-sync, opt-in detach via flag | `nomad job run` (default), `docker run` (default), `fly deploy` | One mental mode; CI must remember `--detach` |
| Two verbs (apply + wait) | `kubectl apply` + `rollout status` | Composable; mental tax to remember both |
| Two verbs (plan + apply) | `terraform plan` + `terraform apply` | Plan-confirm-apply gate; foreign to ops mental model |
| Dense one-shot status | `nomad alloc status` | Full picture but verbose |
| Multi-command status | `kubectl get/describe/logs/events` | Discoverability tax |
| Single-screen status | `systemctl status` | Best for interactive; limited depth |
| Stream status | `kubectl get -w`, `journalctl -f` | Excellent for live inspection; overkill for snapshots |

---

## Non-obvious alternative (per skill diversity requirement)

**`journalctl -fu <unit>` paired with `systemctl start <unit>`** —
different category, same job.

`systemctl start` returns fast; the operator follows the unit's logs
in a separate (often already-open) terminal via `journalctl -fu`.
The orchestration is "fire the command, watch the log stream that was
already open." This decouples *trigger* from *observation* without
forcing the trigger to wait. The status surface IS the log stream,
not a status command.

**Why this matters for the divergence**: an option that ships
convergence-observation as a *streaming surface* (rather than a
snapshot status command) is structurally distinct from every
kubectl/nomad/fly variant. Option A in the options canvas
(`alloc status --follow` as the canonical post-submit move) lifts
this pattern into Overdrive's surface.

---

## Phase 2 gate verdict — G2 PASS

- [x] 3+ real products named (`kubectl apply` / `rollout status`,
  `nomad job run` / `alloc status`, `fly deploy`, plus reference
  points: `docker run`, `systemctl` + `journalctl`, `terraform plan`
  / `apply`).
- [x] Non-obvious alternative included (`systemctl + journalctl -f`
  pattern; `terraform plan/apply` mental model).
- [x] No generic market claims — every comparison cites specific
  commands, specific output shapes, and specific failure modes.
- [x] What each does well, where each fails the job, and the key
  assumption underlying each are all stated.
