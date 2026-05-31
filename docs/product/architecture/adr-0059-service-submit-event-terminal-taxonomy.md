# ADR-0059 — `ServiceSubmitEvent` terminal taxonomy completeness: `Stopped` variant, `ServiceFailureReason::{BackoffExhausted, Other, Timeout, StreamInterrupted}`, probe-less Service deferred to ADR-0058 inference

## Status

Accepted. 2026-05-26. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-health-check-probes/`.

Tags: phase-1, service-kind, wire-shape, streaming, taxonomy.

**Extends**: ADR-0056 (`ServiceSubmitEvent` V1→V2 wire migration —
introduced `Stable` and `Failed` but left the taxonomy incomplete).
**Companion**: ADR-0055 (`ServiceLifecycleReconciler`), ADR-0057
(TOML spec), ADR-0058 (default-TCP startup probe inference), ADR-0037
(`TerminalCondition`), ADR-0032 (NDJSON streaming + `build_stream`
cap-timer / channel-closed synthesis).

## Context

ADR-0056 landed the `ServiceSubmitEvent::{Accepted, Stable, Failed}`
shape and `ServiceFailureReason::{StartupTimeout, StartupProbeFailed,
EarlyExit, LivenessProbeFailed}` — sufficient for the "happy-path
Service reaches Stable" and "Service fails its startup gate" stories.
A subsequent dispatch wiring `ServiceSubmitEvent` into the production
streaming path (`handlers.rs:498` → `build_stream`) surfaced that the
ADR-0056 variant set is a **strict subset** of the terminal states a
Service-kind workload can produce today. Wiring the dispatch against
the current type would either invent undefined projections or regress
user-visible behavior.

The gap inventory, as observed against today's reconciler /
`build_stream` / spec surface:

| Terminal source | Legacy `SubmitEvent` projection | ADR-0056 `ServiceSubmitEvent` equivalent |
|---|---|---|
| `TerminalCondition::Stable` | n/a (new) | `Stable { settled_in_ms, witness }` ✓ |
| `TerminalCondition::ServiceFailed { reason: Startup* / EarlyExit / LivenessProbeFailed }` | n/a (new) | `Failed { reason, stderr_tail }` ✓ |
| `TerminalCondition::Stopped { by }` (operator `overdrive job stop`) | `ConvergedStopped { alloc_id, by }` | **MISSING** |
| `TerminalCondition::BackoffExhausted { attempts }` (general workload-lifecycle restart budget) | `ConvergedFailed { reason: BackoffExhausted }` | **MISSING** (the existing `ServiceFailureReason::LivenessProbeFailed` is liveness-scoped, not the general attempt-budget shape) |
| `TerminalCondition::Custom { type_name, .. }` (driver / WASM extension fallback) | `ConvergedFailed { reason: Other }` | **MISSING** |
| Wall-clock cap timer expiry (synthesised by `build_stream`) | `ConvergedFailed { reason: Timeout }` | **MISSING** |
| Broadcast channel closed `RecvError::Closed` (synthesised) | `ConvergedFailed { reason: StreamInterrupted }` | **MISSING** |
| Service with no startup probes declared (e.g. `health_check = {}` + exec-only workload) | `ConvergedRunning` once `state == Running` | **NO TERMINAL** — reconciler never emits `Stable`; stream hangs until cap timer |

Two of these surfaces are emitted by the reconciler today
(`Stopped`, `BackoffExhausted`); one is emitted by future reconciler
work but reachable via `TerminalCondition::Custom` from third-party
WASM reconcilers (whitepaper §18); two are synthesised by
`build_stream` itself; one is a spec-shape question that the
reconciler decides by not-deciding (no Stable predicate ever
satisfies).

Conway's Law note: the action shim writes the same `TerminalCondition`
value to both `AllocStatusRow.terminal` and the broadcast
`LifecycleEvent.terminal` per ADR-0037 §4 — so the row surface
already carries `Stopped` / `BackoffExhausted` / `Custom` for
Service-kind workloads. The wire surface MUST project all three or
operators observing via streaming see a stream that terminates
silently (best case) or hangs to cap (worst case) for terminals the
row surface already records correctly.

## Decision

### 1. Q1 — `Stopped` is its own `ServiceSubmitEvent` variant (option 1a)

```rust
// crates/overdrive-control-plane/src/streaming.rs (extension)
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize,
         serde::Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceSubmitEvent {
    Accepted { /* unchanged */ },
    Stable { /* unchanged */ },
    Failed { /* extended; see Q2/Q3 */ },

    /// NEW per ADR-0059 Q1. Operator (or reconciler) stopped the
    /// Service before it reached Stable, OR after Stable. Projects
    /// from `TerminalCondition::Stopped { by }`. Preserves the
    /// `StoppedBy` distinction so the CLI render layer can choose
    /// "stopped by operator" vs "stopped (process exited)" vs
    /// "stopped (by system gc)".
    Stopped {
        alloc_id: String,
        by: overdrive_core::transition_reason::StoppedBy,
    },
}
```

`Stopped` is **not** a `Failed` sub-reason. Rationale:

- **CLI exit-code semantics diverge.** Per ADR-0032 §9 the legacy
  `ConvergedStopped` exits the CLI with code 0 (or 130 for
  SIGINT-stop on the Job path). Folding Stop into `Failed` would
  force the CLI to branch on `ServiceFailureReason::Stopped` to
  recover the exit-0 path, which is structurally indistinguishable
  from "the discriminator IS the exit-code class." The legacy
  `JobSubmitEvent::Stopped` already uses a sibling variant; Service
  matches the Job convention for cross-kind operator UX.
- **Operator mental model.** A Service that an operator explicitly
  stopped did not fail; the wire shape should not call it one.
- **Symmetry with `JobSubmitEvent::Stopped`** already in the codebase
  (streaming.rs:1095). Cross-kind operator UX stays parallel.

Rejected (option 1b): folding into `Failed { reason: Stopped }` —
collapses two distinct exit-code semantics into one taxonomic
bucket. Rejected (option 1c): defer — `TerminalCondition::Stopped` is
already reachable today (`overdrive workload stop` on a Service-kind
intent triggers it); without a variant, every Service-kind stop hangs
the stream until cap.

### 2. Q2 — `ServiceFailureReason::BackoffExhausted` carries a `cause` discriminator

```rust
// crates/overdrive-core/src/transition_reason.rs (extended)
#[derive(/* same derives as ADR-0056 */)]
#[serde(tag = "reason", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ServiceFailureReason {
    // Existing variants from ADR-0056 unchanged:
    StartupTimeout { probe_idx: u32, attempts: u32 },
    StartupProbeFailed { probe_idx: u32, last_fail: String, attempts: u32 },
    EarlyExit { exit_code: i32 },
    LivenessProbeFailed { probe_idx: u32, attempts: u32 },

    /// NEW per ADR-0059 Q2. Service-kind workload exhausted the
    /// general restart-attempt budget (`WorkloadLifecycleView::
    /// restart_counts >= RESTART_BACKOFF_CEILING`). Distinct from
    /// `LivenessProbeFailed` (which fires when liveness consecutive-
    /// failures + restart-budget jointly exhaust).
    ///
    /// `attempts` is the count at the deciding tick. `cause`
    /// disambiguates the budget that ran out:
    ///   * `AttemptBudget` — general post-spawn crash loop (driver
    ///     exit + WorkloadLifecycle restart). Today's emit site.
    ///   * `LivenessBudget` — liveness-probe-driven restart budget
    ///     ran out. Reserved for Phase 2+ when the
    ///     `LivenessRestartGovernor` (ADR-0055 §7) lands; today the
    ///     `LivenessProbeFailed` variant covers this case
    ///     end-to-end. Including the discriminator now keeps the
    ///     wire shape forward-compatible without splitting the
    ///     variant later.
    BackoffExhausted {
        attempts: u32,
        cause: BackoffCause,
        last_exit_code: Option<i32>,
    },
}

#[derive(/* rkyv + serde + ToSchema; non_exhaustive */)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BackoffCause {
    AttemptBudget,
    LivenessBudget,  // Phase 2+ surface; defined now, emitted later
}
```

**Disambiguation answer.** Today, `TerminalCondition::BackoffExhausted
{ attempts }` is emitted only by `WorkloadLifecycle` (the general
crash-loop budget). `LivenessProbeFailed` is emitted only by
`ServiceLifecycleReconciler` (liveness-driven path). The two
reconcilers' code paths are disjoint; the projection function at the
wire boundary picks the variant based on the source
`TerminalCondition`:

- `TerminalCondition::BackoffExhausted { attempts }` →
  `Failed { reason: ServiceFailureReason::BackoffExhausted { attempts,
  cause: BackoffCause::AttemptBudget, last_exit_code: <observation>
  } }`. The `last_exit_code` is read from the latest
  `AllocStatusRow.exit_code` at projection time (same surface
  ExitObserver populates per ADR-0037 §4); this is observation, not
  derived state — the row already exists by the time the projection
  fires.
- `TerminalCondition::ServiceFailed { reason: LivenessProbeFailed }`
  → `Failed { reason: ServiceFailureReason::LivenessProbeFailed
  { ... } }` (unchanged). When the Phase 2+ governor lands, it MAY
  re-shape its terminal to `BackoffExhausted { cause:
  LivenessBudget }` if the disambiguation becomes operator-facing;
  the variant exists today as a forward-compat field, NOT as a
  Phase 1 emission.

Rejected: reusing `LivenessProbeFailed` for the general budget —
LivenessProbeFailed carries a `probe_idx`, which doesn't exist for
a general crash-loop. Splitting later would be a SemVer-major
rename; the cause-discriminator approach is additive and lets the
governor (when it lands) reuse `BackoffExhausted`.

### 3. Q3 — `ServiceFailureReason::Other` carries the verbatim driver text (option 3a)

```rust
ServiceFailureReason::Other {
    /// CamelCase identifier the source reconciler emitted. For
    /// platform terminals this is the WASM third-party
    /// reconciler's canonical name (`type_name` from
    /// `TerminalCondition::Custom`); for platform-source unknowns
    /// this is `"platform.unknown"`.
    source: String,
    /// Verbatim driver / reconciler text. Operators see this in
    /// the CLI render; the structured cause lives in the row's
    /// `TerminalCondition::Custom.detail: Option<Vec<u8>>`.
    message: String,
}
```

Projection: `TerminalCondition::Custom { type_name, detail }` →
`Failed { reason: ServiceFailureReason::Other { source: type_name,
message: <rendered from detail or empty> } }`.

**Lossy-encoding choice.** `TerminalCondition::Custom.detail` is
`Option<Vec<u8>>` (opaque rkyv-encoded bytes per the reconciler's
private payload); the wire `message` field is `String`. The
projection renders `detail` via best-effort UTF-8-or-hex (utf8 if
the bytes are valid UTF-8; lowercase-hex otherwise — both are
operator-renderable). The byte-equality contract from ADR-0037 §4
applies to the typed `TerminalCondition`, NOT to the wire
projection — the row carries the full opaque bytes, the wire
carries a renderable summary. Operators who need the structured
payload inspect the row via `overdrive alloc status` (ADR-0033).

Rejected (option 3b): mapping `Custom` to `EarlyExit { exit_code:
-1 }` — invents an exit code the workload never produced; misleads
the operator about the cause. Rejected (option 3c): defer —
`TerminalCondition::Custom` is reachable today via the
forward-compat ADR-0037 §1 variant set; without a projection the
wire surface for any Custom-emitting reconciler would close
silently or hang to cap.

### 4. Q4 — `Timeout` and `StreamInterrupted` are `ServiceFailureReason` variants (option 4a)

```rust
ServiceFailureReason::Timeout {
    after_seconds: u32,
}
ServiceFailureReason::StreamInterrupted,  // unit variant
```

Both surfaces survive on the Service-kind wire. The cap-timer
synthesis is operator-visible per ADR-0032 §"streaming cap"
(60 s default; informs the operator "we waited this long"); the
channel-closed synthesis is the structurally-correct response to
server-side shutdown (a Service whose stream never terminated is
worse UX than a Service whose stream terminated with "interrupted").

Projection sites (synthesised by `build_stream`, NOT projected from
`TerminalCondition` — these are pure streaming-loop terminals with
no reconciler analogue):

- Cap-timer arm → `ServiceSubmitEvent::Failed { alloc_id: None,
  reason: ServiceFailureReason::Timeout { after_seconds }, stderr_tail:
  None }`.
- `RecvError::Closed` arm → `ServiceSubmitEvent::Failed { alloc_id:
  None, reason: ServiceFailureReason::StreamInterrupted, stderr_tail:
  None }`.

**Note on `alloc_id: None`.** The existing `ServiceSubmitEvent::
Failed` shape already accepts `alloc_id: Option<String>`
(streaming.rs:1146); both synthesised terminals carry `None` because
neither the cap-timer arm nor the channel-closed arm has the alloc
in scope. The `Stable` variant by contrast carries
`alloc_id: String` (required) because Stable emission always
originates from a per-alloc reconciler decision.

**Decision discipline.** These two variants are **streaming-loop-
synthesised only**. The reconciler MUST NOT emit them; if a future
ADR adds a reconciler-driven timeout, it adopts a new variant
(`ReconcilerTimeout { ... }`) rather than overloading the
streaming-side `Timeout`. The semantic difference matters: streaming
`Timeout` says "the operator's wait budget elapsed; the reconciler
may still converge"; a reconciler-driven timeout would say "the
reconciler gave up." Conflating them via shared variant would lie
about the underlying state. The wire surface enforces the
separation by leaving `Timeout` documented as cap-timer-only and
rejecting any reconciler-emit code path at review time (no
structural enforcement — this is convention plus PR review).

Rejected (option 4b): silent stream close — operators today rely on
the explicit Timeout/StreamInterrupted signal; the CLI distinguishes
"server is dead" from "we waited long enough." Removing it
regresses operator UX for slow-warming Services. Rejected (option
4c): mixed — splitting cap vs channel-closed into different
variants is asymmetric without justification; both are
streaming-loop-internal, both fail the wire.

### 5. Q5 — Probe-less Service is already handled by ADR-0057+ADR-0058; reconciler emits inferred-witness Stable for the explicit-opt-out case (option 5a, scoped)

Re-reading ADR-0057 + ADR-0058 closely: the dispatch's probe-less
gap is narrower than the original Q5 framing suggested. The
existing spec surface already closes most of it:

1. **Spec has no `[[health_check.startup]]` AND has listeners** —
   ADR-0058 §1 inference rule fires: a default TCP probe on
   `listeners[0]` is synthesised; reconciler proceeds normally.
   No gap.
2. **Spec has no `[[health_check.startup]]` AND `listeners` is
   empty** — `ServiceSpec` parse fails per ADR-0058 §5
   (`ParseError::NoListeners`). No submit, no gap.
3. **Spec has explicit `[[health_check.startup]] = []` (empty
   array opt-out)** — operator explicitly asked for "no startup
   probe". This is the only remaining degenerate case.

**Decision for case 3.** The reconciler emits
`TerminalCondition::Stable { settled_in_ms: 0, witness:
ProbeWitness { probe_idx: 0, role: "startup", mechanic_summary:
"none (opted out)", inferred: false } }` immediately on
observing `state == Running`. The operator's empty-array opt-out
IS their explicit acceptance of "Running == Stable" semantics for
this Service — the platform honours the request rather than
hanging the stream.

**Why this does NOT re-introduce RCA-A**: RCA-A was about the
**default** being wrong. ADR-0058 fixed the default; the opt-out
is the operator's deliberate choice to accept the pre-fix
semantic for this specific Service. ADR-0058 §4 already names this
shape: *"Operators who genuinely want first-Running-IS-Stable
semantics opt out explicitly via the empty array. The default is
the trustworthy path."* This ADR concretises that — the opt-out
path's terminal IS `Stable` with a witness recording the opt-out
explicitly (`mechanic_summary: "none (opted out)"`, `inferred:
false`). Operators reading `alloc status` see they opted out.

**Reconciler change**: `ServiceLifecycleReconciler::reconcile`
gains a branch before the existing Stable predicate:

```rust
// NEW branch — must precede the AND-of-all-probes-Pass branch.
if spec.startup_probes.is_empty()
    && fact.state == AllocState::Running
    && !view.stable_announced.contains(alloc_id)
{
    let settled_in_ms = settled_in_ms_from(tick.now_unix,
                                           fact.started_at_unix_ms);
    let witness = ProbeWitness {
        probe_idx: 0,
        role: "startup".to_string(),
        mechanic_summary: "none (opted out)".to_string(),
        inferred: false,
    };
    actions.push(Action::FinalizeFailed {
        alloc_id: alloc_id.clone(),
        terminal: Some(TerminalCondition::Stable {
            settled_in_ms, witness,
        }),
    });
    next_view.stable_announced.insert(alloc_id.clone());
    continue;
}
```

No new variant on `TerminalCondition` / `ServiceFailureReason` /
`ServiceSubmitEvent` is introduced for Q5. The existing `Stable`
variant carries the opt-out semantics; the witness's
`mechanic_summary` is the structural marker. CLI render decorates
`witness.mechanic_summary == "none (opted out)"` as `"stable (no
startup probe declared)"` for operator-facing render parallel to
the ADR-0058 `(inferred)` decoration.

**Implementation upstream**: ADR-0055 §3 (`decide_per_alloc`
priority order) is amended with the new pre-Stable branch. ADR-0058
is NOT amended — the inference path is untouched.

Rejected (option 5b): reject probe-less at submit — paternalistic;
overrides the operator's explicit opt-out, breaks ADR-0058 §4's
documented contract that empty-array IS the opt-out path.
Rejected (option 5c): defer to ADR-0058 expansion — ADR-0058 is
already complete; the gap was in the reconciler's handling of the
already-supported opt-out, not in the inference rule.

### 6. Q6 — Roadmap integration (option 6b, corrective step)

Insert a **single corrective step** between the prior
wire-migration step (which landed 2ec1eb7f) and the walking-skeleton
closure step. The corrective step:

- Lands the `ServiceSubmitEvent::Stopped` variant (Q1) and the
  projection from `TerminalCondition::Stopped`.
- Lands the four new `ServiceFailureReason` variants
  (`BackoffExhausted`, `Other`, `Timeout`, `StreamInterrupted`)
  and the `BackoffCause` discriminator enum (Q2/Q3/Q4).
- Lands the projections from `TerminalCondition::{BackoffExhausted,
  Custom}` to `ServiceFailureReason::{BackoffExhausted, Other}`.
- Lands the `build_stream` Service-kind branch's cap-timer and
  channel-closed synthesis to the new `Timeout` / `StreamInterrupted`
  variants (Q4).
- Lands the `ServiceLifecycleReconciler::reconcile` opt-out
  pre-Stable branch (Q5) — emits `Stable` immediately on
  `state == Running` when `spec.startup_probes.is_empty()`,
  carrying `ProbeWitness.mechanic_summary == "none (opted out)"`.
- **Does NOT land** the actual `handlers.rs:498` dispatch wiring
  (that's the previously-blocked step; it becomes unblocked
  immediately after this corrective step lands).

DES log re-entry: the previously-stopped dispatch-wiring step
(slice 03 or equivalent — naming per the active roadmap) becomes
the **next** step after this corrective step lands. No re-execute
of 2ec1eb7f. The corrective step adds to the taxonomy; the
follow-up step wires the now-complete taxonomy into the production
dispatch path.

Rejected (option 6a): amend the prior step in-place — DES log
contract treats committed steps as immutable; re-executing 2ec1eb7f
corrupts the log. Rejected (option 6c): fold into walking-skeleton
— walking-skeleton fixtures exercise the happy path; they don't
exercise Stop / general BackoffExhausted / Custom / cap / closed /
probe-less. Folding hides the taxonomy expansion behind unrelated
work. Rejected (option 6d): partial taxonomy + documented
restriction — operators submitting probe-less Service specs today
already see hanging streams; shipping the walking skeleton against
the partial taxonomy ships the bug as a feature.

**Step shape (proposed)**: name `service-submit-event-taxonomy-
completeness` (or equivalent under the active roadmap); ~80 LOC
type additions, ~150 LOC projection logic, ~250 LOC acceptance
tests covering each new projection path. Single PR. Mutation gate
applies to the new projection branches.

## Considered alternatives

### Alternative A — Amend ADR-0056 in-place

Add an "Amendment 2026-05-26 — wire taxonomy completeness" section
to ADR-0056. Rejected: ADR-0056's narrow identity (V1→V2 single-cut
migration with Stable + Failed variants) is muddied by six new
question-resolutions answered post-acceptance. A new ADR cross-
referencing ADR-0056 preserves the original's decision provenance.

### Alternative B — One mega-enum: `ServiceSubmitEvent::Terminal { reason }`

Collapse all terminal cases (Stable, Failed, Stopped, Timeout,
StreamInterrupted) into a single `Terminal { reason: ... }` variant
parameterised by a fat enum. Rejected: the variant identity IS the
operator-facing classification (per ADR-0037 §3 SemVer convention
mirroring K8s `Condition.Reason`). Operators reason about "the
Service stopped" vs "the Service failed" vs "the Service became
Stable" as distinct events; collapsing forces every consumer to
match on the inner `reason` discriminator. Variant identity at the
outer layer is the load-bearing taxonomy.

### Alternative C — Project `Custom` as a new top-level
`ServiceSubmitEvent::Extension { source, payload_summary }` variant

Give third-party WASM reconciler terminals their own wire variant.
Rejected: third-party `Custom` is reachable in Phase 1 only via the
forward-compat path (no third-party reconciler exists today); a
top-level variant for a Phase 2+ surface is premature design space.
`ServiceFailureReason::Other` is sufficient — extension-emitted
terminals manifest to operators as "this Service failed for a
reason the platform didn't author"; the operator workflow is the
same as for any other failure.

### Alternative D — Use existing `JobSubmitEvent::Stopped`-style
exit-code semantics on Service `Stopped`

Carry `exit_code: i32` on `ServiceSubmitEvent::Stopped` to match
the Job sibling. Rejected: Service Stop is operator-initiated, not
process-exit-driven; the workload's exit code is observation, not
the cause of the Stop. The `by: StoppedBy` field IS the cause.
Operators wanting the workload's exit code inspect the
`AllocStatusRow` via `overdrive alloc status` (ADR-0033).

### Alternative E — Leave the empty-array opt-out hanging the stream

Ship the taxonomy expansion without the reconciler's pre-Stable
opt-out branch; let operators who declare `[[health_check.startup]]
= []` see the cap-timer `Timeout` and infer they need to add a
probe. Rejected: ADR-0058 §4 documented the empty-array shape as
the **intentional opt-out path** for operators wanting first-
Running-IS-Stable semantics. Leaving it hanging silently breaks
that documented contract. Per `.claude/rules/development.md`
§"Distinct failure modes get distinct error variants," cap-timeout
should mean "we waited; reconciler hadn't converged"; an operator
who opted out of probes shouldn't see "we waited for nothing."

## Consequences

### Positive

- **Wire taxonomy matches reconciler/streaming surface.** Every
  terminal a Service can reach today has a wire projection;
  dispatch wiring is unblocked.
- **Operator UX preserved.** `Stopped` exits CLI with code 0;
  `Timeout` / `StreamInterrupted` carry their distinct semantics;
  `Other` carries the vendor-reconciler diagnostic.
- **Probe-less Service degeneracy closes structurally** — submit
  rejects fast with a clear error rather than hanging the stream.
- **Forward-compat for Phase 2+ governor** — `BackoffCause`
  discriminator lets the future `LivenessRestartGovernor` reuse
  `BackoffExhausted` rather than splitting the variant later.
- **Greenfield single-cut** per project convention — no transition
  shim, no deprecation cycle.

### Negative

- **`ServiceFailureReason` grows from 4 to 8 variants** plus the
  `BackoffCause` discriminator. The schema-evolution test fixture
  for `TerminalCondition` (which embeds `ServiceFailureReason`)
  regenerates against the new canonical layout per the greenfield
  single-cut exception in `.claude/rules/development.md`
  §"rkyv schema evolution". No prior consumers carry persisted
  bytes against the pre-existing layout; existing fixtures pin
  the canonical post-this-ADR shape.
- **`ServiceLifecycleReconciler::reconcile` gains a pre-Stable
  branch** for `spec.startup_probes.is_empty()`. The existing
  probe-less TOML parser test stays green (parser-layer); a new
  reconciler acceptance test covers the opt-out Stable emission
  with `mechanic_summary == "none (opted out)"`.
- **Two streaming-loop-synthesised variants** (`Timeout`,
  `StreamInterrupted`) are emitted only from `build_stream`, not
  from any reconciler. The convention is documented in §4 above;
  enforcement is PR-review-level (no structural lint).
- **Lossy `Custom.detail` → wire `message` projection.** Operators
  who need the full opaque payload read the row via `alloc status`.
  Acceptable — the wire is a notification surface, not a forensic
  surface.

### Quality-attribute impact

| Attribute | Impact |
|---|---|
| Functional correctness | Every terminal projects; no silent stream hangs |
| Compatibility — evolvability | Additive variants per ADR-0037 §5; `BackoffCause` non-exhaustive for Phase 2+ |
| Maintainability — modifiability | One enum, one projection site, one mapping function |
| Reliability — surface coherence | Row + wire surfaces project from the same source `TerminalCondition`; byte-equality preserved where applicable |
| Usability — operator UX | Distinct exit-code / render semantics preserved across Stopped / Failed / Stable / Timeout |

## Cross-references

- ADR-0032 — NDJSON streaming, cap-timer + channel-closed synthesis
- ADR-0037 — `TerminalCondition` SemVer convention; this ADR adds
  no new `TerminalCondition` variants (the gap was wire-side only)
- ADR-0055 — `ServiceLifecycleReconciler`; consumer of the wire
  projections via the action-shim broadcast
- ADR-0056 — `ServiceSubmitEvent` V1→V2 wire migration (this ADR
  extends)
- ADR-0057 — `[[health_check.*]]` TOML; spec-layer optionality
- ADR-0058 — default-TCP startup probe inference; Q5 above
  concretises ADR-0058 §4's "operators opt out via empty array"
  contract by defining the opt-out Stable shape (no amendment;
  ADR-0058's inference rule is unchanged)
- ADR-0055 — `ServiceLifecycleReconciler::reconcile` priority
  order; **this ADR amends ADR-0055 §3** with the new pre-Stable
  empty-probes opt-out branch (Q5)
- `feedback_single_cut_greenfield_migrations.md` — no transition
  shim
- `feedback_phase1_single_node_scope.md` — `LivenessBudget`
  `BackoffCause` is Phase 2+ forward-compat only

## Changelog

- 2026-05-26 — Initial accepted version. Resolves Q1–Q6 surfaced
  by the dispatch-wiring blocker following commit 2ec1eb7f.
