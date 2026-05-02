# Options (raw, unevaluated) — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DIVERGE / Phase 3
**Owner**: Flux (`nw-diverger`)
**Date**: 2026-04-30
**Discipline**: Generation only. No evaluation. No "this is the best
one." Curation to 6 happens after all generation completes; scoring is
done in `taste-evaluation.md`.

---

## HMW question

> **How might we make the time and reasoning between declaring an
> intent and knowing whether the platform converged on it as small as
> possible, without forcing operators to learn a separate diagnostic
> toolchain?**

This is broad enough that "block on submit," "stream from a separate
verb," "richer status snapshot," and "structured event log" are all
plausible answers, and narrow enough to forbid drift into "rebuild the
whole CLI."

No embedded solution. Outcome-oriented. Positively framed.

---

## SCAMPER lens generation

### Option S — Substitute: Submit-streams-default

**Core idea**: One verb. The wait *is* the submit. `overdrive job
submit ./job.toml` writes intent, then **streams the lifecycle
reconciler's evaluation** for this specific job over the same HTTP
request, printing each transition as it lands in the ObservationStore.
The command returns when the job reaches a terminal-ish state for this
submit (≥1 replica `Running`, or `Failed` with backoff exhausted, or a
configurable wall-clock budget elapsing). `--detach` short-circuits
the stream and returns after the IntentStore commit. TTY detection on
stdout auto-applies `--detach` semantics in piped contexts.

**Key mechanism**: HTTP long-poll or SSE on the submit response;
server holds the connection open and writes ObservationStore deltas
filtered to this job's evaluation chain.

**Key assumption**: Operators want the inner-loop verb to do the
obvious thing; CI users will pass `--detach` (or the TTY-detection
will catch them).

**SCAMPER origin**: Substitute — replaces submit-then-poll with
submit-then-stream-inline.

**Closest competitor**: `nomad job run` (default-wait), `fly deploy`.

---

### Option C — Combine: Submit + watch (combined session)

**Core idea**: `overdrive job submit ./job.toml` defaults to opening
an interactive convergence pane. The spec digest scrolls in, then the
lifecycle reconciler's transitions stream into a small TUI table
(allocation states, last transition reason, restart counts). Ctrl-C
exits the pane; the job continues reconciling in the background.

**Key mechanism**: Same long-poll/SSE backend as Option S, but
client-side renders into a structured TUI via `ratatui`.

**Key assumption**: Operators are okay with submit being an
interactive TUI experience by default; `--quiet` or `--detach` exists
for non-TTY.

**SCAMPER origin**: Combine — merges the submit verb with the
observation verb into one interactive session.

**Closest competitor**: `lazygit`, `k9s`, `fly deploy`'s progress
display (but TUI-shaped, not line-shaped).

**Note on consolidation**: Option C shares the long-poll mechanism
with Option S; the differentiator is the TUI client renderer. During
curation (below) I considered merging C into S as a future-extension
note. C survives as a distinct option because the *interaction model*
(drop into TUI vs read line stream) is structurally different from
the operator's perspective, and the TUI's accessibility/dependency
cost is non-trivial.

---

### Option A — Adapt: Submit-async + `alloc status --follow`

**Core idea**: Two verbs. Submit returns the way it does today
(commit_index, hint). The hint changes from `alloc status --job ...`
to `alloc status --follow --job ...`, and `--follow` (or `--watch`)
**streams lifecycle transitions as they happen**. The operator's
muscle memory becomes "submit, then follow" — exactly mirroring
"`systemctl start` and then `journalctl -fu`," which they already
have in their hands. The non-`--follow` `alloc status` remains a
snapshot but is materially richer than today (state, node, resources,
last reason, started/transition timestamps).

**Key mechanism**: `alloc status` becomes both a snapshot endpoint
and a streaming endpoint; the streaming mode subscribes to the
ObservationStore via long-poll/SSE.

**Key assumption**: Senior platform engineers already have the
`systemctl start` + `journalctl -f` pattern in their muscle memory
and will adopt the analogous shape immediately.

**SCAMPER origin**: Adapt — borrows the well-established
`systemctl + journalctl -f` operator pattern.

**Closest competitor**: `kubectl get pod -w`; `systemctl start` +
`journalctl -fu <unit>`; `kubectl rollout status` (but on a generic
status verb rather than a rollout-specific one).

---

### Option M — Modify/Magnify: Submit-async + dense `alloc status` snapshot

**Core idea**: Submit stays fire-and-forget (today's behaviour
unchanged). `alloc status` is **rebuilt as a dense single-screen
render** that shows: spec digest, desired vs running replicas,
per-allocation state with last transition reason, recent driver
events (last 5–10), last reconciler tick timestamp, and
restart-budget state. Not streaming — a snapshot, but a *useful*
snapshot. Operators can `watch -n 1 overdrive alloc status` if they
want continuous updates, mirroring the universal Unix pattern.
Optional `--wait` flag on submit can be added as a depth-revealed
extension.

**Key mechanism**: Snapshot endpoint expanded to surface
ObservationStore + reconciler private libSQL events. **No new
transport protocol** — just richer JSON in the existing
`alloc status` response.

**Key assumption**: The submit/wait split is fine; the bug is
*purely* "alloc status is too thin." Refusing to ship streaming/wait
machinery in Phase 1.

**SCAMPER origin**: Modify/Magnify — amplifies the most important
dimension (status output density and honesty) without changing the
verb structure.

**Closest competitor**: `systemctl status <unit>` (single-screen
dense render); `nomad alloc status` (one-shot dense).

---

### Option P — Put to other use: New `deploy` verb

**Core idea**: Add `overdrive deploy <spec>` as the canonical verb
for "submit and wait until convergence or failure." Keep `overdrive
job submit` for the rare automation case that wants raw commit
semantics. Position `deploy` as the top-recommended verb in `--help`;
the inner loop becomes `overdrive deploy ./job.toml`. Different from
Option C because deploy is a **separate verb** (not a flag on submit)
and the wait is non-interactive — line-by-line progress like `fly
deploy`.

**Key mechanism**: `deploy` server-side: submit the spec, then
attach to the lifecycle reconciler's stream for the just-submitted
job, render line-by-line progress.

**Key assumption**: Operators want a named "make this real" verb;
submit's `Accepted.` semantics aren't *wrong*, they're just the wrong
default for the inner loop. The verb-choice signal is a clearer
mental model than a flag.

**SCAMPER origin**: Put to other use — borrows the app-platform
deploy verb (heroku, fly, render) and adapts it to a control-plane
context.

**Closest competitor**: `fly deploy`, `heroku deploy`, `terraform
apply` (in spirit; in shape it's closest to fly).

---

### Option E — Eliminate: One-verb (submit absorbs status)

**Core idea**: Eliminate `alloc status` entirely. Make `submit` the
only operator-facing convergence verb. `submit` waits by default,
prints lifecycle transitions, and exits non-zero on convergence
failure. There is no `alloc status`. If the operator wants to inspect
a running job later, they re-run `submit` (idempotent — the
IntentStore commit is a no-op if the spec digest hasn't changed) and
the wait reconnects to the live convergence stream. Maximum
subtraction: one verb, one experience, no second mental model for
"how do I check on a job."

**Key mechanism**: Submit is idempotent on spec digest; re-running
attaches to the live stream of the existing job.

**Key assumption**: Re-running submit to inspect is acceptable for
the inner loop; for "I want to look at a running job not currently
being submitted" the operator uses logs / cluster status instead.
The "sometimes-commit-sometimes-attach" dual semantic is
discoverable.

**SCAMPER origin**: Eliminate — removes the most complex part
(`alloc status` + the verb-choice cognition).

**Closest competitor**: None directly. Closest is `docker run`
(no `docker check-on-my-running-container`; you `attach` or `logs`),
but that breaks because docker workloads detach naturally; Overdrive
allocations don't have a "foreground" semantic.

---

### Option R — Reverse: Plan / Apply split

**Core idea**: `overdrive plan ./job.toml` is a new pre-submit verb:
it dry-runs the spec against the local control plane (validates
types, runs the scheduler in dry-run mode against current node
capacity, runs admission policy if any) and **prints what would
happen** — "would schedule on local node, would request 2GiB / 2
cores, binary at `/usr/local/bin/payments` is missing." `overdrive
apply ./job.toml` actually commits + waits. The reverse: instead of
"submit and discover failures asynchronously," the operator sees the
failures *before* the IntentStore is touched.

**Key mechanism**: Plan = validate spec + dry-run scheduler + dry-run
driver pre-checks (e.g., `stat` the binary path) without IntentStore
commit. Apply = the current submit + a wait phase.

**Key assumption**: Operators value the "no surprises" property of
seeing the plan first. Pre-flight checks (binary exists, capacity
sufficient, ports available) cover the common-case failures so wait
is rarely needed.

**SCAMPER origin**: Reverse — flips the workflow so failure
discovery happens *before* commit, not after.

**Closest competitor**: `terraform plan` + `terraform apply`;
`kubectl apply --dry-run=server`.

---

## Crazy 8s supplements

Two additional options generated under time pressure to break out of
SCAMPER's bounded set. Each was tested for structural distinctness
from the SCAMPER options.

### Option X1 — `--wait` flag on submit

**Core idea**: Single verb. Default fire-and-forget (today's
behaviour). `--wait` (with optional `--timeout=Ns`,
`--for=running|terminal`) blocks. `alloc status` is the snapshot
surface; `alloc status --watch` is the streaming surface (subset of
A). Three orthogonal tools, operator picks per invocation.

**Key mechanism**: Flag-gated wait on submit; richer alloc-status
snapshot; watch flag for stream.

**Diversity verdict**: **Merged into Option M** as the optional
extension. M's "fix status, don't add streaming" plus an optional
`--wait` flag covers the same surface area; X1 doesn't introduce a
new mechanism. M's option description notes the optional `--wait` as
a depth-revealed extension.

### Option X2 — TTY-detected auto-stream on submit

**Core idea**: Submit always streams convergence by default (like S),
BUT detects non-TTY stdout and auto-detaches in that case. Pure
UX-detection: interactive operator sees the stream; CI pipe sees the
fire-and-forget shape they see today. Ctrl-C aborts the *stream* but
keeps the reconciliation running.

**Key mechanism**: TTY detection on stdout; if interactive → stream;
if piped → detach.

**Diversity verdict**: **Merged into Option S** as a sub-feature.
S's mechanism is identical; X2 just adds isatty branching to the
client. S's description carries the TTY-detection note.

---

## Curation to 6

After merging X1 → M and X2 → S, **seven candidates remain**: S, C,
A, M, P, E, R. The skill targets six. The closest pair is S and C —
both share the long-poll mechanism, differing only in client
renderer (line stream vs structured TUI).

**Decision**: Drop **C** as a standalone; carry its TUI dimension as
a future-extension note inside Option S. Rationale: S is the
lower-cost variant with the same mechanism, and the TUI dimension is
genuinely a future-phase concern (ratatui dependency, accessibility
audit, non-TTY fallback) that does not need to be decided in this
divergence. The TUI extension can be added on top of S without
re-architecting.

**Final 6 curated options**: S, A, M, P, E, R.

---

## Diversity test on the 6

Per the skill: each of the 6 must answer yes to (a) different
mechanism, (b) different assumption, (c) different cost profile.

| # | Different mechanism? | Different assumption? | Different cost profile? |
|---|---|---|---|
| **S — Submit-streams-default** | Long-poll on submit endpoint; server pushes lifecycle deltas | Inner loop dominates; sync default is correct; CI uses `--detach` | HTTP streaming on `/v1/jobs` POST; isatty branch; backend filter to job-id |
| **A — Submit-async + `alloc status --follow`** | Long-poll on alloc-status endpoint | Two-verb composition matches existing systemctl + journalctl muscle memory | HTTP streaming on `/v1/alloc/status`; richer snapshot; submit unchanged |
| **M — Submit-async + dense status snapshot** | Snapshot only; richer aggregation; no streaming | Submit/wait split is fine; status was the bug | No new protocol; aggregate from ObservationStore + libSQL into existing JSON; submit unchanged |
| **P — `deploy` verb** | New verb mounted on a new HTTP path | Named verb signals operator intent better than a flag | Same backend as S, different path; OpenAPI surface +1; submit kept |
| **E — One-verb (submit absorbs status)** | Submit becomes idempotent and stateful; status verb removed | Re-submit-to-inspect is acceptable; verb subtraction wins | Major submit-handler rework; `alloc status` code paths removed; idempotency machinery added |
| **R — Plan/Apply split** | Pre-flight dry-run; commit only after the operator confirms | Operators want plan-before-apply; pre-flight catches most failures | New `plan` endpoint; dry-run scheduler; dry-run driver `stat` checks; new mental model |

All six pass: distinct mechanism, distinct primary assumption,
distinct cost profile. None are degree-variations of another.

---

## Eliminated options (curation note)

| Option | Reason eliminated |
|---|---|
| C — TUI submit | Merged into S as a future-extension note. Same mechanism (long-poll); the TUI cost is non-trivial (ratatui, accessibility, non-TTY fallback) and is a future-phase concern. |
| X1 — `--wait` flag on submit | Merged into M as the optional `--wait` extension. Same fundamental position ("fix status, don't change submit fundamentally") plus a flag. |
| X2 — TTY-detected auto-stream | Merged into S as a sub-feature. Same long-poll mechanism plus isatty branch. |

---

## Phase 3 gate verdict — G3 PASS

- [x] HMW question framed without embedded solution.
- [x] All seven SCAMPER lenses applied (S, C, A, M, P, E, R) — one
  option per letter.
- [x] Crazy 8s supplements generated (X1, X2) and merged with
  rationale.
- [x] 6 curated options after deduplication and merging.
- [x] Each of the 6 passes the 3-point diversity test.
- [x] No evaluation language in option descriptions — each option is
  described by mechanism, assumption, SCAMPER origin, and closest
  competitor only.
