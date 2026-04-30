# JTBD Analysis — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DIVERGE / Phase 1
**Owner**: Flux (`nw-diverger`)
**Date**: 2026-04-30

---

## 1. Raw request (verbatim)

> I deliberately submitted a job with an exec command referencing a non-existing binary, but shouldn't job submission wait for it to successfully start, or should that be part of a separate command, e.g "deploy"?
>
> "alloc status" also doesn't show anything remotely useful

**Session evidence** (from the user's reproduction):

- `overdrive job submit ./job.toml` → `Accepted.` plus a spec digest, commit
  index, and a `Next: overdrive alloc status --job payments-v2` hint. The
  submission returned successfully even though the spec's `exec.command`
  referenced a non-existent binary path.
- `overdrive alloc status --job payments-v2` → `Allocations: 1`. That
  was the entirety of the rendered output — no state, no node, no
  driver error, no exit code, no convergence reason.

The two questions are tightly coupled. The shape of "should submit
wait?" dictates what `alloc status` must show, and the shape of
`alloc status` dictates whether a non-blocking submit is even
acceptable.

---

## 2. Job extraction (5 Whys)

| # | Question | Answer |
|---|---|---|
| 1 | Why does the user expect submit to wait, or want a separate `deploy` verb? | Returning `Accepted.` while the workload silently fails to start is dishonest. The platform "accepted" something that cannot run; the operator must discover failure asynchronously through a separate command. |
| 2 | Why is asynchronous discovery painful here? | J-OPS-003 already commits the platform to "honest about what it does and does not know — no silent blank outputs, no fabricated placeholder rows." The current shape forces the operator to invent a polling loop AND simultaneously discover that the polling target doesn't show actionable information. Two failures stack. |
| 3 | Why does that compound failure matter to a senior platform engineer? | They run this as the inner loop of "is my spec correct?" — a tight edit-submit-observe-fix cycle. Every additional step where the platform forces them to dig (open another terminal, parse logs, check `journalctl`, kubectl-describe-style) is a tax on the loop. They will categorise the platform as "not trustworthy" if the first deliberately-broken submit produces zero diagnostic signal. |
| 4 | Why does "trustworthy" matter at this stage? | Phase 1 is the walking-skeleton test. Overdrive's case rests on "honest behaviour, kernel-evident, bit-identical, ESR-verified." If the operator's first deliberately-broken submit returns `Accepted.` with no follow-up, that promise is already dented in the introductory experience. |
| 5 | Why should the platform owe the operator anything before they ask for status? | The primary inner-loop *activity* is "make the platform run my workload," not "make the platform accept my paperwork." Submit accepting the spec is necessary but not sufficient — the operator's actual progress is "is it running, and if not, why not?" |

---

## 3. Job statements

### Functional (primary)

> When I submit a job spec to the local control plane during the inner
> loop of getting a workload to run, I want to know within seconds
> whether the workload converged or failed, and if it failed, why and
> where, so that I can fix the spec or the binary and try again
> without context-switching to other terminals or external tools.

### Emotional

> Feel that the platform is honest about what it knows and doesn't
> know — no false success, no silent failures, no fabricated rows.
> Trust the CLI to be telling me the actual state of the world.

### Social (operator self-image)

> Be the kind of platform engineer whose tooling makes failure modes
> obvious to anyone watching over my shoulder, not a tooling user who
> has to apologise "well, you also have to look at /var/log/..."
> while debugging.

---

## 4. Abstraction-layer check

Per the JTBD skill, jobs must live at strategic or physical level — not
tactical.

| Layer | Statement | Verdict |
|---|---|---|
| Tactical | "Make `submit` block / make `alloc status` better" | Where the user's literal request lives. **Not the job.** |
| Operational | "Compress the edit-submit-observe-fix loop on a single workload" | Closer. |
| **Strategic** | **"Reduce the time and uncertainty between declaring intent and knowing whether the platform converged on it"** | **The job.** |
| Physical | "Reduce the entropy between operator-mental-model and platform-actual-state" | Higher abstraction; sanity floor. |

The strategic level is the right layer. It is **solution-agnostic**: it
does not specify "blocking submit," "watch flag," "deploy verb,"
"richer status output," or "plan/apply" — and all six structural
options in `options-raw.md` are different mechanisms that serve this
same job.

---

## 5. Disruption check

Is there a higher-level job that would make this entire job
unnecessary? **No.** The platform's whole purpose is "declare intent,
converge actual." This job is foundational to the platform's
operator-facing existence.

---

## 6. ODI outcome statements

Format: `[Direction] + [Metric] + [Object] + [Context]`. All
"Minimize"-direction; no embedded solutions; no compound statements.

1. **Minimize the time it takes** to know whether a submitted spec
   converged to its declared running state during the inner-loop
   edit-submit-observe-fix cycle.
2. **Minimize the likelihood of** a submitted spec being silently
   accepted while the workload it declares cannot start.
3. **Minimize the time it takes** to identify the specific reason a
   submitted spec failed to converge (binary not found, capacity
   exceeded, permission denied, etc.).
4. **Minimize the effort required to** observe convergence transitions
   (Pending → Running, or Pending → Failed) without invoking external
   tools.
5. **Minimize the likelihood of** the operator having to re-derive
   convergence state from CLI output that omits state, node,
   resources, or failure reason.
6. **Minimize the time it takes** to distinguish "platform hasn't
   converged yet" from "platform converged to a failure state."

---

## 7. Opportunity scoring

Importance and Satisfaction estimated from the validated J-OPS-003 +
the senior-platform-engineer persona. Score = `Importance + max(0,
Importance − Satisfaction)`.

| # | Outcome | Importance | Satisfaction | Score | Status |
|---|---|---|---|---|---|
| 1 | Time to know if spec converged | 9.5 | 2.0 | 17.0 | **Severely under-served** |
| 2 | Likelihood of silent-accept-while-failing | 9.0 | 1.5 | 16.5 | **Severely under-served** |
| 3 | Time to identify convergence failure reason | 9.0 | 1.0 | 17.0 | **Severely under-served** |
| 4 | Effort to observe transitions without external tools | 8.0 | 2.5 | 13.5 | Under-served |
| 5 | Likelihood of re-deriving state from sparse output | 8.5 | 1.5 | 15.5 | **Severely under-served** |
| 6 | Time to distinguish "not yet" from "failed" | 9.0 | 2.0 | 16.0 | **Severely under-served** |

Five of six outcomes score above 15 — consistent with the user's
"doesn't show anything remotely useful" framing. Outcome 4 is somewhat
better-served because the *existence* of `alloc status` is at least
documented; the rest are unmet by definition.

---

## 8. Constraints carried into brainstorming

These are not part of the job; they are the structural constraints any
option must respect.

- **Phase 1 = single-node.** No node registration, no multi-region, no
  scheduler placement choice to surface. The "node" is implicit.
- **Intent / Observation split is non-negotiable** (whitepaper §4).
  Submission writes intent through the linearizable IntentStore;
  convergence is observed asynchronously through the ObservationStore.
  The lifecycle reconciler runs on its own tick. This is the structural
  reason a naïve "submit blocks until Running" cannot work — but a
  *wait* mode that **subscribes to observation** is fine.
- **CLI is HTTP/REST against the local control plane** (ADR-0008). No
  bidirectional streaming yet; long-poll, server-sent events (SSE), or
  polling subscription patterns are the realistic shapes for a "wait"
  mode in Phase 1.
- **Reconciler purity** (§18, ADR-0023). The lifecycle reconciler
  cannot perform I/O inside `reconcile`; the CLI's wait mechanism
  consumes from the ObservationStore (which the reconciler writes
  through `Action::*`-shaped emissions), not from the reconciler
  directly.
- **Greenfield migration discipline.** If a direction renames `submit`
  to `apply` or splits `submit` from `deploy`, the cut is single — no
  deprecation period, no feature-flagged old paths.

---

## Phase 1 gate verdict — G1 PASS

- [x] Job at strategic level — "reduce time/uncertainty between intent
  and convergence-knowledge."
- [x] No solution references in job statement (no mention of
  blocking, waiting, deploy verb, watch flag, plan/apply).
- [x] Six ODI outcome statements produced (minimum is 3).
- [x] Five of six outcomes scored as severely under-served — high
  opportunity, validating that the user's complaint is well-grounded.
