# Recommendation — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DIVERGE (handoff to DISCUSS)
**Owner**: Flux (`nw-diverger`)
**Date**: 2026-04-30
**Decision target**: product-owner (DISCUSS wave) ratifies the
direction; DESIGN owns the HTTP/CLI shape ADRs that follow.

---

## TL;DR

**Recommended direction**: **Option S — Submit-streams-default.**
`overdrive job submit ./job.toml` becomes a single verb that writes
intent and then streams the lifecycle reconciler's convergence
inline, exiting non-zero on convergence failure. `--detach` for CI;
TTY detection auto-detaches in piped contexts. `alloc status` is
kept as a snapshot surface and is **separately enriched** to match
the journey-extended TUI mockup (state, node, resources, last
transition reason, restart-budget).

This answers both halves of the user's question coherently:

1. **"Should submit wait?"** — Yes, by default, with a flag to opt
   out. The wait IS the submit; there is no separate `deploy` verb.
2. **"Alloc status doesn't show anything useful"** — Independent
   bug, fixed by making the snapshot surface dense (the journey
   extension already specifies the exact shape).

Score: **4.47** vs. runner-up **3.77** (Option A). Clear winner, not
a coin flip.

---

## Context

The user, a senior platform/SRE engineer, deliberately submitted a
job spec whose `exec.command` referenced a non-existing binary, and
reported two complaints in the same breath:

> shouldn't job submission wait for it to successfully start, or
> should that be part of a separate command, e.g "deploy"? "alloc
> status" also doesn't show anything remotely useful

The session showed:

- `overdrive job submit ./job.toml` returned `Accepted.` with
  spec-digest + commit-index + a hint to `overdrive alloc status`,
  despite the binary being absent.
- `overdrive alloc status --job payments-v2` rendered only
  `Allocations: 1` — no state, no node, no resources, no driver
  error.

The two questions are tightly coupled; the answer to "should submit
wait?" dictates what `alloc status` needs to show, and vice versa.

The validated job, captured in JTBD analysis at the strategic level:

> Reduce the time and uncertainty between declaring intent and
> knowing whether the platform converged on it.

Six structurally distinct options were generated and scored against
locked taste weights (DVF 25%, T1 Subtraction 15%, T2 Concept Count
25%, T3 Progressive Disclosure 15%, T4 Speed-as-Trust 20%). T2 was
raised 5pp from the dev-tool default (concept count is the most
diagnostic lens for a verb-soup risk); T4 was lowered 5pp (the user's
complaint is about honesty, not raw speed). See `taste-evaluation.md`
for the full matrix.

---

## Top 3

### #1 — Option S — Submit-streams-default — 4.47

#### Why it scores well

- **T1 Subtraction (5)**: One verb does the inner-loop job. There is
  no second "wait" or "follow" verb to learn. The wait IS the submit.
- **T3 Progressive Disclosure (5)**: First interaction is type
  submit, watch convergence happen. Depth (status snapshot, retry
  budget, cluster status) is revealed only on demand.
- **T2 Concept Count (4)**: One concept (submit) plus a minor flag
  (`--detach`). Maps cleanly to `nomad job run`, `fly deploy`, and
  `docker run` — operator muscle memory most senior SREs already
  have. TTY detection means many operators won't even encounter the
  flag.

#### Core trade-off

Submit becomes a long-lived HTTP request (Phase 1: HTTP long-poll or
SSE). The operator's terminal is occupied for the duration of
convergence — typically sub-second on a healthy spec, several seconds
on a backoff-bounded failure. CI scripts must pass `--detach` (or
benefit from TTY detection if their stdout is piped, which is the
common case). A single-binary submit verb that holds a connection
open is a structural change to the existing `POST /v1/jobs` shape.

#### Key risk (the assumption that must be true)

**Operators want a single-verb inner-loop experience and accept that
the verb's success criterion is "converged," not just "committed."**
Senior platform engineers raised on `kubectl` (default-async) may
need a brief acclimation, but the user's literal framing of the
question ("shouldn't submit wait?") is itself evidence of this
assumption. The fallback for non-interactive use is well-precedented
(`--detach`).

#### Hire criteria — under what circumstances would a user choose S?

When they want one verb that tells the truth on the first command. The
inner-loop case is dominant; CI is the exception, served by a flag
the operator already expects to pass for any "run" command in the
ecosystem.

#### Happy-path shape

```
$ overdrive job submit ./payments.toml
Submitted spec sha256:7f3a... at commit 42.
  payments-v2/a1b2c3   pending     scheduling on local
  payments-v2/a1b2c3   pending     starting (pid 12345)
  payments-v2/a1b2c3   running     pid 12345, 2000mCPU / 4 GiB

Job 'payments-v2' is running with 1/1 replicas (took 1.4s).
$ echo $?
0
```

#### Broken-binary shape (the user's actual session, fixed)

```
$ overdrive job submit ./payments.toml
Submitted spec sha256:7f3a... at commit 42.
  payments-v2/a1b2c3   pending     scheduling on local
  payments-v2/a1b2c3   pending     starting (pid attempt 1)
  payments-v2/a1b2c3   failed      driver: stat /usr/local/bin/payments: no such file or directory
  payments-v2/a1b2c3   pending     starting (pid attempt 2; backoff 5s)
  payments-v2/a1b2c3   failed      driver: stat /usr/local/bin/payments: no such file or directory
  ...

Error: job 'payments-v2' did not converge to running.
  reason: driver start failed (binary not found)
  last-event: stat /usr/local/bin/payments: no such file or directory
  reproducer: overdrive alloc status --job payments-v2

Hint: fix the spec's `exec.command` path and re-run.
$ echo $?
1
```

Two of the six ODI outcomes are met inline, in one verb, in seconds:

- **Outcome 1** — time to know if spec converged (sub-second to
  several seconds) ✓
- **Outcome 3** — time to identify failure reason (printed in the
  same output) ✓
- **Outcome 6** — distinguish "not yet" from "failed" (exit code +
  explicit `Error:` line) ✓

`alloc status` (kept and enriched per Option M's snapshot dimension)
serves outcomes 4 and 5 for second-day inspection.

#### Phase 1 implementation cost

- `POST /v1/jobs` returns `application/x-ndjson` (one JSON event per
  line) when `Accept: application/x-ndjson` is requested by the CLI;
  retains `application/json` for backward compatibility with raw
  scripts.
- **API contract evolution is the load-bearing design call.** The
  endpoint's shared-type surface (per ADR-0014, `overdrive-control-
  plane::api`) must model two response shapes: the existing
  `SubmitJobResponse` JSON for `Accept: application/json`, and an
  `enum SubmitEvent { Committed { … }, AllocStateChanged { … },
  Terminal { outcome: Outcome, … } }` NDJSON stream for `Accept:
  application/x-ndjson`. This is more design surface than the
  4-feasibility score naively suggests, but the patterns are mature
  (NDJSON in production at observability vendors; axum + reqwest both
  ship streaming primitives). The DESIGN-wave HTTP-shape ADR (below)
  owns this explicitly.
- Server holds the connection while subscribing to the
  ObservationStore for the just-committed `JobId`'s lifecycle events.
- Connection closes when the lifecycle reconciler reports a terminal
  state (Running ≥ desired or Failed-with-backoff-exhausted) OR a
  configurable wall-clock budget elapses.
- CLI: isatty(stdout); if not a TTY, send `Accept: application/json`
  and behave like today; if a TTY, send `Accept:
  application/x-ndjson` and stream-render. `--detach` overrides to
  the JSON path.
- Backend reuses the lifecycle reconciler's existing event emissions
  through the action shim (ADR-0023). No new I/O surface in the
  reconciler itself; the streaming endpoint is a consumer of the
  ObservationStore.
- ESR / DST: the streaming endpoint is a synchronous consumer of
  observation rows; no new clock or RNG surfaces are introduced.

#### Forward-compatibility notes

- A future TUI mode (Option C dimension) can ride the same NDJSON
  stream, rendered into `ratatui` instead of stdout lines. No
  protocol change.
- A future `:start` companion verb (per ADR-0027's verb-suffix
  pattern) follows the same shape: `POST /v1/jobs/{id}:start` returns
  NDJSON convergence events.
- Because submit returns NDJSON only when requested, automation
  consuming the JSON shape continues to work as today (with
  `Accept: application/json` or no Accept header).

---

### #2 — Option A — Submit-async + `alloc status --follow` — 3.77

#### Why it scores well (and why it's runner-up, not winner)

- **T2 Concept Count (3)**: Two verbs is more than one verb (S has
  4); the second verb is a flag on an existing one (`--follow`),
  which softens the cost.
- **T3 Progressive Disclosure (4)**: Sequencing is explicit: submit,
  then follow. Hint at the end of submit points at the follow
  command. Mirrors `systemctl start` + `journalctl -fu` muscle memory.
- **T1 Subtraction (3)**: Two verbs survive. S's single-verb wins
  the subtraction lens.

#### Core trade-off

Submit returns immediately (today's shape), exit 0. Whether the
workload actually starts depends on whether the operator runs the
follow command. Some operators will paste the output, see `Accepted.`
and a hint, and move on without following — leaving the broken
spec running its backoff in the background. Honest snapshot still
catches them on the next `alloc status` call, but the failure is
delayed by one operator decision.

#### Key risk

**Operators read the hint and execute it.** Most do; some don't.
The "submit returns successfully and then nothing happens" failure
mode (the user's actual session) is mitigated by hint-driven
discovery, not eliminated. The hint must be rendered prominently
enough that ignoring it requires effort.

#### Hire criteria

When the team is uncomfortable with submit holding a connection
open, or wants to keep the existing `POST /v1/jobs` HTTP shape
unchanged. A team committed to keeping verb count steady at 2 (submit
+ alloc-status) and unwilling to converge them.

#### Phase 1 implementation cost

- `alloc status` gains a `--follow` flag.
- New endpoint or content-type variant on
  `GET /v1/alloc/status?job=...` to support streaming.
- Submit endpoint unchanged.
- Snapshot surface still needs the dense-render work (this is shared
  with M and is a no-regret improvement regardless of which option
  ships).

---

### #3 — Option M — Submit-async + dense-status snapshot — 3.68

#### Why it scores well (and why it's third, not first)

- **T1 Subtraction (4)**: Doesn't add anything new. No new verb, no
  new flag if `--wait` is deferred.
- **T3 Progressive Disclosure (4)**: First interaction unchanged
  from today; second is a richer snapshot.
- **T4 Speed-as-Trust (2)**: **The fatal cost.** Submit still
  returns `Accepted.` for a broken binary. The user's literal first
  question ("shouldn't submit wait?") is structurally unaddressed.

#### Core trade-off

This option *only* fixes the second half of the user's complaint
("alloc status doesn't show anything useful"). The submit-vs-deploy
question is answered "no, submit doesn't wait, the operator polls."
Operators are forced to invent their own polling loop or run
`watch -n 1 overdrive alloc status`, which is what the user was
already doing implicitly when they got `Allocations: 1`.

#### Key risk

**The operator can tolerate the polling-by-hand model if status is
honest enough.** The journey-extended TUI mockup is dense and would
be sufficient for second-day inspection. For the first-time
inner-loop case (the user's session), polling is still required and
the broken-spec failure is still discovered asynchronously.

#### Hire criteria

When the team explicitly does not want to ship streaming machinery
in Phase 1. A "minimum viable status fix" position. Keeps the door
open for adding `--wait` to submit later.

#### Phase 1 implementation cost

- No new transport protocol.
- `alloc status` JSON response shape grows to carry events,
  restart-budget, last-tick timestamp, per-allocation reason.
- Submit unchanged.

---

## Eliminated options (taste-phase)

### Option E — One-verb (submit absorbs status) — 3.58 (4th)

Maximum subtraction (T1=5) but the dual-mode "sometimes-commit-
sometimes-attach" semantic creates a new concept that costs T2.
"How do I check on yesterday's job?" surfaces a re-run-submit answer
that contradicts senior-SRE expectations.

### Option P — `deploy` verb — 3.13 (5th)

Adds a third verb without justifying it (Overdrive doesn't have
fly's build-and-push complexity). T1=2 and T2=2 are the lowest of
all options after R.

### Option R — Plan/Apply split — 2.96 (6th, last)

Best speed-as-trust score (T4=5 — failures surface before any state
mutation) but worst on every other lens. Foreign mental model for
ops (terraform shape vs kubectl/nomad shape). Forces three concepts
on first interaction. Strong long-term shape; wrong fork for this
divergence.

---

## Dissenting case

**The runner-up Option A could be chosen instead** if the team's
priority weight on T4 (Speed-as-Trust, the honesty lens) were lower
or the priority weight on operational stability higher. The case for
A:

- **Streaming submit is a structural change.** It binds submit's
  HTTP shape to the lifecycle reconciler's tick budget. A flaky
  reconciler tick will manifest as a "stuck" submit in the user's
  terminal. The journey-extended emotional arc requires sub-second
  responsiveness; a 30-second backoff window during convergence
  makes submit feel slow even when the platform is behaving correctly.
- **Two-verb composition is well-precedented.** `kubectl apply` +
  `kubectl rollout status` is muscle-memory for half the user base.
  The hint-driven discovery model works in production at Kubernetes'
  scale; there is no honest reason to claim it cannot work for
  Overdrive.
- **Honest snapshot still catches the broken-binary case.** The
  journey extension already specifies the snapshot's failure-mode
  output ("Failed: <reason>"). An operator who runs `alloc status`
  after a broken submit gets the same diagnostic information they
  would get from S's stream — just one command later.
- **A is cheaper to ship in Phase 1.** Submit endpoint is unchanged;
  `alloc status` gains streaming as a flag. The work is bounded; S
  requires changing a load-bearing endpoint's response shape.

If the DISCUSS wave decides that streaming on submit is too
structural a change for Phase 1, **Option A is the correct fallback**.
The decision then has the explicit shape: "we will not block submit;
we will instead make `alloc status --follow` the canonical
post-submit move and ensure the hint is loud enough that operators
cannot miss it."

---

## Decision for DISCUSS

> **Proceed with Option S (submit streams convergence by default;
> `--detach` for CI; TTY detection auto-detaches piped contexts;
> `alloc status` enriched as a dense snapshot per the journey-
> extended TUI mockup), assuming we accept that submit becomes a
> long-lived HTTP request bounded by the lifecycle reconciler's
> convergence-or-backoff window.**

**Fallback if the DISCUSS wave rejects the streaming-submit assumption**:
Option A — submit unchanged; `alloc status --follow` becomes the
canonical post-submit move; the hint at the end of submit's output is
made loud and unambiguous; status snapshot is enriched per the same
TUI mockup.

**Sharp trigger for the fallback**: choose Option A if **either** of
the following holds:
1. The team will not ship streaming machinery (NDJSON / SSE on a
   load-bearing endpoint) in Phase 1.
2. The API contract evolution from sync-JSON to polymorphic-by-Accept-
   header is judged too expensive for the Phase 1 deadline (see
   "Phase 1 implementation cost" under Option S above).

If neither holds, Option S is the recommended direction.

**Both options share** the snapshot-enrichment work on `alloc status`.
That work is no-regret regardless of which way the wait question
goes, and DESIGN can begin on it before the wait-shape ADR is
finalised.

**Out of scope for this divergence**: any changes to `cluster
status`, `logs`, the OpenAPI schema's discoverability, or the
ProcessDriver's pre-flight surface. The plan/apply pre-flight idea
(Option R) is rejected for Phase 1 but is worth keeping on the
backlog as a Phase 2+ consideration once the multi-step rollout
verbs (`:start`, `:restart`, `:cancel-pending-rollout`) start
landing.

**ADR follow-on (DESIGN wave's responsibility)**:

1. HTTP shape ADR: streaming response on `POST /v1/jobs` (NDJSON or
   SSE) gated by `Accept` header; CLI behaviour matrix (TTY vs piped
   vs `--detach`).
2. `AllocStatus` snapshot enrichment ADR: which fields surface, how
   the lifecycle reconciler's libSQL-tracked retry-budget is
   exposed, and the rendering contract for the CLI's snapshot
   output.

Both follow ADR-0027's precedent for the HTTP shape and ADR-0014's
shared-types discipline.
