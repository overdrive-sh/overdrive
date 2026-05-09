# Wave Decisions — workload-kind-discriminator (DISCUSS)

**Wave**: DISCUSS
**Feature**: workload-kind-discriminator
**Date**: 2026-05-09
**Author**: Luna (nw-product-owner)

## Configuration captured at dispatch

| Field | Value |
|---|---|
| feature_id | `workload-kind-discriminator` |
| feature_type | cross-cutting (CLI / spec parser / control-plane streaming / `alloc status` render) |
| walking_skeleton | No (extends a landed Phase 1 walking skeleton) |
| jtbd_analysis | SKIPPED — covered by J-OPS-002 / J-OPS-003 in `docs/product/jobs.yaml`; taxonomy validated by `docs/research/platform/workload-type-taxonomy-research.md` |
| research_depth | comprehensive |
| format | all (visual + yaml + gherkin) |
| output_directory | `docs/feature/workload-kind-discriminator/discuss/` |

## Entry context (no DISCOVER / DIVERGE waves)

This feature originates from an operator-observed bug, not a new opportunity:

1. **Bug RCA** — `docs/analysis/root-cause-analysis-coinflip-submit-reports-running-on-exit-1.md`
   names four composing root causes (A–D) for `overdrive job submit examples/coinflip.toml`
   reporting `Job 'coinflip' is running with 1/1 replicas (took live)` while the underlying
   workload exits with status 1. The conjunction of B+C+D becomes structurally unreachable
   for a Job-kind workload after this feature lands.
2. **Industry research** — `docs/research/platform/workload-type-taxonomy-research.md` cross-
   validates a three-aggregate model (Service/Job/Schedule) across 13/15 vendor primaries.
3. **Convergence transcript** — `/tmp/attachments/pasted_text_2026-05-09_23-40-44.txt`
   captures the spec-shape negotiation that landed on section-as-discriminator
   (`[service]` / `[job]` / `[job]+[schedule]`) and the operator framing for `alloc status`
   ("when we check the status using overdrive alloc status --job <id> then it should show
   that it failed during execution").

The DISCOVER wave is intentionally absent — DISCOVER frames opportunity from open user
research; this feature has a closed problem statement (the bug) and a closed solution
direction (the taxonomy). Re-running DISCOVER would manufacture optionality the system does
not have.

The DIVERGE wave is intentionally absent — DIVERGE selects between candidate solution
directions; this feature has one direction validated by 13 vendor primaries, with the
remaining choice (TOML shape) already converged in the transcript.

## JTBD traceability (DIVERGE-equivalent grounding)

Because JTBD analysis was skipped, every story in `user-stories.md` MUST trace to one of
the existing job statements in `docs/product/jobs.yaml`:

| Job ID | Title | This feature's relevance |
|---|---|---|
| **J-OPS-002** | Submit a job to the walking-skeleton control plane and trust what the CLI tells me | **PRIMARY** — the bug under audit IS a violation of "trust what the CLI tells me"; this feature restores honesty for run-to-completion workloads. |
| **J-OPS-003** | Run my actual workload on the walking-skeleton control plane and trust the platform to converge to the declared replica count | **SECONDARY** — `alloc status` honesty for Failed/Succeeded/Running differs by kind; this feature makes the kind explicit so convergence semantics are kind-aware. |

No new job statement is added. The motivations are downstream of J-OPS-002's "honest about
what it does and does not know — no silent blank outputs, no fabricated placeholder rows"
clause and J-OPS-003's "alloc status honestly reflects whether each allocation is Pending,
Running, Draining, Terminated, or Failed."

## Changed Assumptions vs. RCA / research

- 2026-05-10 — folded in [overdrive-sh/overdrive#164](https://github.com/overdrive-sh/overdrive/issues/164)
  (service listener spec shape) per user approval. Adds Slice 06 / US-08;
  runtime allocator behaviour for `vip = None` is tracked at
  [overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167)
  (approved 2026-05-09). Prior wave-decisions claim of "5 slices, 7 stories"
  is superseded by "6 slices, 8 stories". See § "Fold-in of GH #164" below
  for full converged decisions.

Otherwise, this wave's artifacts remain the operator-facing manifestation of the design
moves R1–R7 recommended in the research; no contradiction surfaced during journey or
story crafting.

The RCA's four root causes (A–D) are **not** addressed in isolation by this feature — A
(no liveness gate after `fork+exec`) remains a separate concern best framed as
"`Service.start` settle window" per research R7. That settle window is out of scope here;
its absence does not block this feature, because for a Service the bug shape is "user gets
a false positive only if the service crashes within the streaming window," which is a
different and rarer failure mode than the Job case this feature targets.

If the DESIGN wave's architect wishes to fold the settle window into this feature's slice
list, they can — but DISCUSS frames it as a separate concern.

## Risks surfaced for DESIGN handoff

| # | Risk | Severity | Owner | Mitigation framing |
|---|---|---|---|---|
| R1 | Renaming current `Job` aggregate to `Service` is repository-wide | Medium | architect | Single-cut migration per `feedback_single_cut_greenfield_migrations.md` — no compat shim, no deprecation; greenfield discipline applies. |
| R2 | Streaming-protocol per-kind split (`ServiceSubmitEvent` vs `JobSubmitEvent`) ripples through wire format | Medium | architect | Phase 1 ships single binary; wire compat is internal, not external. ADR-0032 amendment captures the rename. |
| R3 | `[schedule]` execution semantics (cron parser, fire-on-tick, history retention) are larger than parser validation | High | PO + user | **DEFERRAL**: see § "Deferrals requiring user approval" below. Slice 05 covers parsing + composition validation only. |
| R4 | RCA root cause A (no settle window) is not closed by this feature | Low | architect | Documented above. Service still has a small false-positive window after this feature; remediation is a separate ADR. |
| R5 | DIVERGE artifacts absent — risk that DESIGN wave architect demands them | Low | PO | Not blocking: research doc and RCA together provide the same epistemic content as a DIVERGE pass would. Captured here for traceability. |

## Deferrals (approved + tracked)

### Deferral 1 — Schedule execution semantics — tracked at GH #166

User approved on 2026-05-09; orchestrator created
[overdrive-sh/overdrive#166](https://github.com/overdrive-sh/overdrive/issues/166).

- **Scope**: cron parser, schedule reconciler that emits `Action::SubmitJob` on tick,
  `ConcurrencyPolicy` (Allow/Forbid/Replace), `successfulJobsHistoryLimit` /
  `failedJobsHistoryLimit`, missed-fire `startingDeadlineSeconds`. Slice 05 of this
  feature covers parsing the `[schedule]` block and validating composition rules
  ("`[schedule]` only valid alongside `[job]`"); it does NOT cover *executing* the
  schedule.
- **Affected areas**: a future `ScheduleLifecycle` reconciler in
  `crates/overdrive-control-plane/src/reconciler/`, a `CronExpr` newtype in
  `crates/overdrive-core/src/`, schedule-aware streaming events on the submit path,
  schedule-aware `alloc status` rendering.
- **Slice 05 wiring**: the CLI constant `SCHEDULE_EXECUTION_TRACKING_URL` MUST equal
  `https://github.com/overdrive-sh/overdrive/issues/166` byte-for-byte (KPI K5 asserts
  the submit-echo and `alloc status` deferral notices reference the same URL).

This is the only execution-side deferral. The runtime VIP allocator
(referenced from Slice 06) is tracked separately at #167 — see § "Fold-in
of GH #164" below; it is NOT a deferral OF this feature, it is a separate
follow-up feature whose ABSENCE the spec shape is forward-compatible with.

## Fold-in of GH #164 (service listener spec shape)

User explicitly approved folding [overdrive-sh/overdrive#164](https://github.com/overdrive-sh/overdrive/issues/164)
into this DISCUSS wave on 2026-05-10. Converged decisions are recorded in
[#164's comment](https://github.com/overdrive-sh/overdrive/issues/164#issuecomment-4413120509)
and reproduced here for the DESIGN handoff:

### Locked design decisions

- **Section name**: `[[listener]]` — top-level array-of-tables alongside
  `[service]`. NOT `[[backend]]`, which would collide with the dataplane's
  existing destination-address `Backend` type.
- **`vip` field**: `Option<ServiceVip>`. When `Some(addr)`, the parser
  validates IPv4 syntax. When `None`, the spec layer carries `None` forward
  and the CLI render layer prints the literal `(vip: pending allocation —
  see #167)`. The runtime decision (allocate at runtime vs. reject at
  admission) is OUT OF SCOPE for this feature; tracked at #167.
- **Field name**: `protocol`, NOT `proto`. Matches Kubernetes terminology;
  improves operator readability. Case-insensitive parsing; canonical render
  is lowercase.
- **Listener uniqueness**: no two `[[listener]]` blocks within a single
  Service may share `(vip, port, protocol)`. When both `vip` are `None`,
  comparison is on `(port, protocol)` only.
- **At least one listener required**: a Service with zero `[[listener]]`
  blocks is rejected with named guidance.
- **`Proto` newtype reuse**: the spec layer imports
  `overdrive-core::Proto` (the kernel-side enum already used by the
  dataplane). No second copy.
- **Supported protocols**: `tcp`, `udp` (Phase 2.2 surface). `sctp`,
  `icmp`, and any other value rejected with named guidance.

### Tracked follow-up — runtime allocator at GH #167

User approved on 2026-05-09; orchestrator created
[overdrive-sh/overdrive#167](https://github.com/overdrive-sh/overdrive/issues/167).

- **Scope**: runtime VIP allocator behaviour when a Service spec declares
  `vip = None`. Open decision: allocate-at-runtime vs. reject-at-admission.
  Slice 06 of THIS feature ships only the spec shape, which is
  `Option`-shaped and forward-compatible with either #167 outcome.
- **Slice 06's wiring**: the literal `(vip: pending allocation — see #167)`
  is sourced from a single CLI config constant (architect to name —
  candidate: `SERVICE_VIP_ALLOCATOR_TRACKING_URL`); both submit echo and
  `alloc status` listener-line render paths read from this constant so the
  two surfaces cannot drift.

### Updated handoff scope for nw-solution-architect

Adds to the existing handoff list:

- **ADR-0031 amendment** — extend the `[exec]` block placement amendment
  to ALSO cover the new top-level `[[listener]]` array-of-tables.
- **Possible new ADR** — "Service listener fields (port, protocol, optional
  VIP)". Architect's call: fold into the workload-kind-discriminator ADR
  (research R4 working title) as a "Service listener fields" section, or
  ship as its own ADR. Either way: the runtime allocator decision belongs
  to #167, not this ADR.

## Handoff readiness

See `dor-validation.md` for the 9-item DoR pass (8/8 stories pass as of
2026-05-10). See `user-stories.md` for the 8 stories. See `slice-NN-*.md`
files for the 6 carpaccio slices.

The DESIGN wave (`@nw-solution-architect`) should expect:

- One new ADR (working title from research R4: "Workload kind discriminator — Service /
  Job / Schedule as separate aggregates") — supersedes or amends ADR-0019, ADR-0031,
  ADR-0032, ADR-0033, ADR-0037 in scope-bounded ways.
- ADR-0019 amendment (TOML format) — section-as-discriminator validation rules.
- ADR-0031 amendment (`[exec]` block) — preserved across kinds; placement of `[exec]` and
  `[resources]` at top level, not nested under `[job]` / `[service]`.
- ADR-0032 amendment (NDJSON streaming submit) — per-kind `SubmitEvent` enums
  (`ServiceSubmitEvent`, `JobSubmitEvent`).
- ADR-0033 amendment (alloc status snapshot enrichment) — kind-aware render branches
  (Service: replicas; Job: exit_code + Succeeded/Failed; Schedule: next-fire +
  last-run-result).
- ADR-0037 amendment (reconciler emits typed terminal condition) — Job-kind reconciler
  emits Completed{exit_code: 0} or Failed{exit_code: N} as terminal; Service-kind retains
  current "alive in steady state" semantics.

All priorities (P1, P2, …) are in scope by default per CLAUDE.md.

## Changelog

- 2026-05-09 — Initial DISCUSS wave decisions captured.
- 2026-05-10 — Folded in GH #164 (service listener spec shape) per user
  approval. Added Slice 06 / US-08 / KPI K6. Recorded converged design
  decisions and tracked follow-up at GH #167. DoR pass count updated from
  7/7 to 8/8.
