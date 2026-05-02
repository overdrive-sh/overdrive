# cli-submit-vs-deploy-and-alloc-status — Feature Evolution

**Feature ID**: cli-submit-vs-deploy-and-alloc-status
**Branch**: `marcus-sa/phase1-first-workload`
**Duration**: 2026-04-30 (single-day DIVERGE → DELIVER); finalised 2026-05-02
**Status**: Delivered (9 / 9 steps green; final COMMIT 2026-04-30T18:05:36Z)
**Journey extension**: rewrites step 1 (Submit) of `submit-a-job` into 1 + 1a + 1b + 1c
(streaming-default with intent-ack first line, lifecycle stream, terminal block) and
rewrites step 4 (Inspect) into a dense snapshot (state, resources, started timestamp,
exit code, last transition reason+source, restart budget). Steps 2/3/5/6/7 inherited
unchanged from the prior `phase-1-first-workload` extension.

---

## What shipped

A coherent answer to the user's two-part complaint from the prior session — `overdrive
job submit ./payments.toml` returning `Accepted.` for a job whose binary did not exist,
and `overdrive alloc status --job payments-v2` rendering only `Allocations: 1` for the
same broken allocation. The feature lands three coordinated changes:

- **NDJSON streaming submit by default.** `overdrive job submit` against a TTY now
  negotiates `Accept: application/x-ndjson` and renders the lifecycle reconciler's
  convergence inline, line-by-line. The first line is an `Accepted` event carrying
  `spec_digest` / `intent_key` / `outcome` (the existing JSON ack, lifted onto the wire
  as the first NDJSON record). Subsequent lines are `LifecycleTransition` events from
  the action shim's row writes. The stream closes on `ConvergedRunning` (exit 0),
  `ConvergedFailed` (exit 1), or a server-side wall-clock cap (60 s default; structured
  `Timeout` terminal event; exit 1).
- **Auto-detach + explicit `--detach`.** Stdout-is-not-a-TTY (`overdrive job submit
  ./payments.toml | jq -r .spec_digest`) sends `Accept: application/json` automatically
  and behaves identically to the pre-feature shape. Explicit `--detach` always wins —
  CI scripts that legitimately allocate a TTY can opt out without relying on the
  heuristic. CLI exit codes are 0 / 1 / 2; the sysexits.h range stays reserved.
- **`alloc status` snapshot enrichment.** `AllocStatusResponse` extended in place with
  `replicas_desired` / `replicas_running` / `restart_budget` at the envelope level and
  `state` / `resources` / `started_at` / `exit_code` / `last_transition` / `error` per
  row. The CLI renderer produces the journey YAML's TUI mockup verbatim, with explicit
  `Pending: no node has capacity (...)` rows on the empty case (never silent
  `Allocations: 0`). The `last_transition.reason` is byte-equal to the streaming
  surface's `LifecycleTransition.reason` for the same allocation — the [C6]
  single-source-of-truth pin enforced structurally by routing both surfaces through
  `AllocStatusRow.reason: Option<TransitionReason>`.

The broken-binary case the user originally surfaced now exits 1 with the verbatim
driver error in the streaming output AND in the snapshot's `error:` line, with
`Restart budget: 5 / 5 used (backoff exhausted)` rendered when the lifecycle reconciler
has given up retrying.

## Business context

This feature is a UX divergence on top of the working execution layer landed in
`phase-1-first-workload`. That feature's `JobLifecycle` reconciler converges
allocations and writes `AllocStatusRow`s to the ObservationStore correctly; the
defect was that the operator's two CLI surfaces (`submit` and `alloc status`) both
returned the wrong shape of information for the inner-loop case. Specifically:

1. `submit` returned 200 OK on accepted intent and gave the operator no signal that
   the workload actually started or failed to start.
2. `alloc status` rendered only the allocation count for jobs whose lifecycle had
   already terminated in failure — the row's underlying state, transition reason, and
   driver error were never exposed on the wire.

Together, these gaps meant the operator had to read source to debug a broken spec.
The user's session — submitting a job whose `exec.command` referenced a non-existent
binary, getting `Accepted.`, then getting `Allocations: 1` from status — is the
load-bearing scenario the feature is built around. KPI-02 names it explicitly: a
boolean-defended regression target asserting that the broken-binary submission exits
1 inline with the verbatim driver error reaching both surfaces.

JTBD framing (validated at strategic level, do not re-run): *"Reduce the time and
uncertainty between declaring intent and knowing whether the platform converged on
it."* Six ODI outcomes scored, five severely under-served. Persona Ana — Overdrive
platform engineer with senior SRE muscle memory (`kubectl rollout status`, `nomad
job run`, `fly deploy`, `systemctl start && journalctl -fu`); inner-loop edit-submit-
observe-fix cycle; CI is the secondary case served by `--detach` and TTY auto-detection.

## Wave journey

- **DIVERGE** (2026-04-30) — Flux. JTBD analysis at strategic level (G1 PASS); 3-name
  competitive research (kubectl + rollout, `nomad job run`, `fly deploy`, plus
  systemctl+journalctl as the non-obvious alternative); 6 structurally diverse
  options via SCAMPER; locked taste weights with explicit adjustment (T2 raised 5pp,
  T4 lowered 5pp). **Recommendation: Option S — Submit-streams-default**, score 4.47
  vs runner-up Option A 3.77 (clear winner, 0.70-point gap). Inline peer-review by
  Prism: APPROVED 5/5 dimensions, 2 advisory polishes applied. See
  [`diverge/`](../feature/cli-submit-vs-deploy-and-alloc-status/diverge/).

- **DISCUSS** (2026-04-30) — Luna. Eight key decisions ratified ([D1] NDJSON over
  SSE; [D2] ratify Option S; [D3] exit-code contract 0/1/2; [D4] `alloc status
  --follow` OUT of scope; [D5] server-side wall-clock cap exists, value is DESIGN's
  call; [D6] `AllocStatusResponse` snapshot extension; [D7] single source of truth
  for `transition_reason`; [D8] walking skeleton waived for brownfield extension).
  Six user stories (US-01..US-06); three slice briefs (slice 03 conditional);
  9-item DoR PASS; outcome KPIs KPI-01..KPI-05 mapped to ODI outcomes 1..6.

- **DESIGN** (2026-04-30) — Morgan. Eight ratified decisions, including the
  load-bearing post-discuss amendments to ADR-0032 and ADR-0033 that re-shaped
  `TransitionReason` from a verbatim-text-carrying enum into a **cause-class**
  taxonomy (14 Phase 1 variants; structured payloads via `Arbitrary`-generated
  proptest cases) and `TerminalReason` from a unit enum into a **structured payload**
  enum carrying `cause: TransitionReason` on `BackoffExhausted` and `DriverError`
  variants. Two new ADRs landed (ADR-0032 NDJSON streaming submit; ADR-0033 alloc
  status snapshot enrichment) plus the same-day amendments. Reuse Analysis EXTEND-only
  (zero CREATE NEW unjustified). Echo peer review APPROVED.

- **DISTILL** (2026-04-30) — Quinn. Five DWDs (DWD-01 walking-skeleton waiver
  documented; DWD-02 Tier-1/Tier-3 split — pure-and-property at T1, real-syscall-
  propagation at T3; DWD-03 RED scaffold scope — 5 net-new types scaffolded, 4
  deferred for cross-cutting `DriverType: ToSchema` derive dependency; DWD-04
  Tier-3 Linux-gating + Lima discipline; DWD-05 reuse existing `acceptance/` and
  `integration/` patterns). 26 scenarios catalogued (16 happy-path / 10 error-path,
  ratio ≈ 38%). Three Tier-3 scenarios (`S-WS-01`, `S-WS-02`, `S-CLI-03`); the
  remainder Tier 1.

- **DELIVER** (2026-04-30) — software-crafter via `/nw-deliver`. Roadmap
  approved-with-nits at Phase 2; **9 steps** completed across 3 slices, all GREEN
  on first commit (no escalations, no scaffold-deferred items rejected). Execution
  log shows one mid-step PREPARE re-entry on 03-02 (timestamps 17:13:26Z and
  17:31:33Z) — see *Issues encountered* below.

## Slice-level delivery summary

| Slice | Steps | What shipped |
|---|---|---|
| **Slice 01** — alloc status snapshot enrichment | 01-01, 01-02, 01-03 | `AllocState::Failed` variant; `AllocStatusRow.{reason: Option<TransitionReason>, detail: Option<String>}` field extension (rkyv-additive); cause-class `TransitionReason` enum (14 Phase 1 variants + 2 Phase 2 emit-deferred forward-compat) + supporting types (`StoppedBy`, `CancelledBy`, `ResourceEnvelope`); `TransitionRecord`, `RestartBudget`, `ResourcesBody`, `AllocStateWire`, `TransitionSource` (with `DriverType: ToSchema` derive); `AllocStatusResponse` extension in place; `handlers::alloc_status` rewrite hydrating from `ObservationStore` + `ReconcilerViewCache`; CLI snapshot renderer rewrite producing the journey TUI mockup; honest empty-state for Pending-no-capacity. |
| **Slice 02** — NDJSON streaming submit | 02-01, 02-02, 02-03, 02-04 | `LifecycleEvent` broadcast payload (does NOT carry `AllocStatusRow` — trybuild-enforced); `tokio::sync::broadcast::Sender<LifecycleEvent>` on `AppState`; action-shim `dispatch` emits per row write; **the action-shim classifier** mapping `DriverError::StartRejected.reason` text → cause-class `TransitionReason` variant (the previously-discarded `reason: _` is now the input); `SubmitEvent` enum (4 variants) + `TerminalReason` (3 variants, structured payloads); content negotiation in `submit_job` handler; `streaming_submit_loop` with `tokio::select!` cap timer over injected `Clock`; lagged-recovery snapshot fallback; CLI NDJSON consumer with structured `Error:` block + reproducer hint; broken-binary regression target end-to-end. |
| **Slice 03** — `--detach` + pipe auto-detect | 03-01, 03-02 | `--detach` clap flag forcing `Accept: application/json`; `std::io::IsTerminal::is_terminal(&stdout())` auto-detect with thin trait seam for Tier-1 testability; Tier-3 jq-pipeline scenario (`overdrive job submit ./payments.toml \| jq -r .spec_digest` produces a single 64-hex-char digest line, shell pipeline exit 0); KPI-05 (`--detach` exits ≤ 200 ms p95). |

## Steps completed

| Step | Scenario coverage | Title |
|---|---|---|
| 01-01 | S-CP-09 | Land `AllocState::Failed` + `AllocStatusRow.{reason,detail}` (single source of truth foundation) |
| 01-02 | S-AS-02 | Promote `api.rs` RED scaffolds to GREEN: `TransitionRecord`, `RestartBudget`, `ResourcesBody`, `AllocStateWire` (with `DriverType: ToSchema`) |
| 01-03 | S-AS-01, S-AS-04, S-AS-05, S-AS-06, S-AS-07, S-AS-08, S-AS-09 | Extend `AllocStatusResponse` + rewrite `alloc_status` handler hydration + CLI snapshot renderer |
| 02-01 | S-CP-04 | Land `LifecycleEvent` (broadcast payload) + `broadcast::Sender` on `AppState` + action-shim emit + classifier |
| 02-02 | S-AS-03 | Land `SubmitEvent` enum + `TerminalReason` wire serialization (snake_case) + serde round-trip |
| 02-03 | S-CP-01, S-CP-02, S-CP-03, S-CP-05, S-CP-06, S-CP-07, S-CP-08, S-CP-10 | Wire content negotiation in `submit_job` + `streaming_submit_loop` with `select!` cap timer + lagged-recovery fallback |
| 02-04 | S-WS-01, S-WS-02, S-CLI-04, S-CLI-05 | CLI NDJSON consumer + exit-code mapping + structured `Error:` block + broken-binary regression target |
| 03-01 | S-CLI-01 | Add `--detach` flag on `overdrive job submit` |
| 03-02 | S-CLI-02, S-CLI-03, S-CLI-06 | Auto-detach via `std::io::IsTerminal` + Tier-3 jq-pipeline scenario |

## Key decisions

This wave produced no new ADRs from scratch; instead, two ADRs created on 2026-04-30
(ADR-0032, ADR-0033) carry **same-day amendments** that landed during DESIGN as the
review surfaced shape problems with the original cause-text approach. The amendments
are the load-bearing decisions of the wave.

### ADR-0032 — NDJSON streaming submit shape, with the cause-class amendment

The originally-proposed `TransitionReason` carried verbatim driver error text in a
free-form `String` field. The amendment replaces it with a **structured cause-class
taxonomy**:

- 14 Phase 1 variants split into 5 progress markers (`Scheduling`, `Starting`,
  `Started`, `BackoffPending`, `Stopped`) and 9 cause-class failure variants
  (`ExecBinaryNotFound { path }`, `ExecPermissionDenied { path }`,
  `ExecCgroupSetupFailed { detail }`, `ExecSpawnFailed { detail }`,
  `BackoffExhausted { attempts }`, `NoCapacity { resource_envelope }`,
  `StoppedBy(StoppedBy)`, `CancelledBy(CancelledBy)`, `Unknown { detail }`) plus 2
  Phase 2 emit-deferred forward-compat variants reserved for future driver classes.
- Each variant's payload is structured: `path: PathBuf`, `attempts: u32`,
  `resource_envelope: ResourceEnvelope`, etc. Phase 1's exec-driver classifier
  (action_shim::dispatch_single's L117/L148 rewrite) prefix-matches
  `DriverError::StartRejected.reason` text into the right variant; the verbatim
  text continues to populate `AllocStatusRow.detail` for audit, never the wire-typed
  `reason` field.
- Wire serialisation uses `#[serde(tag = "kind", content = "data", rename_all =
  "snake_case")]`, producing
  `{"kind": "exec_binary_not_found", "data": {"path": "/usr/local/bin/payments"}}`.
  Consumers can match on `kind` for routing and on `data` for rendering.

The corresponding `TerminalReason` shape (originally three unit variants) became
**structured payload**: `BackoffExhausted { attempts, cause: TransitionReason }`,
`DriverError { cause: TransitionReason }`, `Timeout { after_seconds }`. The
`cause: TransitionReason` field on the failure variants is the structural enforcement
of [C6] — the streaming consumer reads the same enum value the snapshot consumer
reads, byte-identical, because the type system makes it so.

### ADR-0033 — alloc status snapshot enrichment, with the renderer mapping table

The amendment to ADR-0033 lands the **rendering contract** for
`TransitionReason::human_readable()` — the operator-facing prose mapping from
cause-class variant to the line shown in the CLI's `Error:` block. The mapping is a
table of one-line strings keyed by variant; for example:

- `ExecBinaryNotFound { path } → "binary not found: <path>"`
- `ExecPermissionDenied { path } → "permission denied: <path>"`
- `BackoffExhausted { attempts } → "restart budget exhausted (<attempts> attempts)"`
- `NoCapacity { .. } → "no node has capacity"`

The CLI renderer also maps cause-class variants to the trailing `Hint:` line —
`ExecBinaryNotFound` and `ExecPermissionDenied` get the existing
`fix the spec's exec.command path and re-run` hint; other variants get neutral hints.
This is what makes the broken-binary scenario the user originally hit produce
*actionable* output rather than just *honest* output.

### Architectural decisions ratified in DESIGN

| ID | Decision | Why |
|---|---|---|
| **D1** | `SubmitEvent` is a flat enum with 4 variants (`Accepted`, `LifecycleTransition`, `ConvergedRunning`, `ConvergedFailed`); structured `reason: TransitionReason` on the lifecycle / failure variants | Enum-level dispatch is what the CLI branches on for exit codes (Running → 0, Failed → 1); the cause is rendering data, not dispatch data. Per-cause discriminated union (A2) would balloon variant count and invert the contract. |
| **D2** | `AllocStatusResponse` extended in place; existing field set is sparse, every new field is `Option` or has a sensible default | Splitting into a per-allocation endpoint (B3) duplicates handler code at Phase 1 cardinality (`replicas=1`); replacing (B2) buys nothing because there is nothing to delete. |
| **D3** | Streaming wall-clock cap = 60 s; handler-local `tokio::select!` over injected `Clock`; configurable via `[server].streaming_submit_cap` | Lifting the timer to a tower layer (C2) is YAGNI for one streaming endpoint; pushing into the subscription primitive (C3) entangles unrelated concerns. The handler is the natural owner; `select!` cancellation on terminal events is natural. |
| **D4** | Subscription via `tokio::sync::broadcast` from the action shim; lagged-subscriber recovery via one-shot `ObservationStore` snapshot fallback | Polling (D2) is in tension with the 200 ms KPI-01 budget and forces diff-derivation across snapshots. The action shim is already the single async I/O boundary that witnesses every state transition; broadcasting from where the writes happen is mechanical. |
| **D5** | CLI-side `IsTerminal::is_terminal(&stdout())` + explicit `--detach`; `--detach` always wins; server stays Accept-driven | Reference class is uniform — `docker run -d`, `nomad job run --detach`. Server-side detection (E3) is wrong on multiple axes (no signal the server can use that the CLI cannot; least-surprise inversion). |
| **D6** | No new endpoint; `POST /v1/jobs` polymorphic on `Accept` header | Two-endpoints-diverge-over-time is the well-known failure mode; ADR-0008's REST shape commits to content-negotiation as the polymorphism mechanism. |
| **D7** | `restart_budget.max = 5` hard-coded in Phase 1 (matches existing `RESTART_BUDGET_MAX`); configurable in Phase 2+ when right-sizing reconcilers land | Surfacing a config hook now without a configuration mechanism is over-engineering. |
| **D8** | OpenAPI declares both `application/json` and `application/x-ndjson` response media types on `submit_job`; vendor extension `x-ndjson-stream: true` is informational | utoipa 5.x supports multiple response media types via the `responses(...)` macro; the existing `cargo xtask openapi-check` gate covers the addition unchanged. |

### Single-source-of-truth pin (the [C6] story, structurally)

The same physical `TransitionReason` enum value flows through:

```
DriverError::StartRejected.reason (verbatim text)
  ↓ classify_driver_failure(text, driver, &spec.command)
TransitionReason (structured cause-class variant)
  ↓ action_shim writes to AllocStatusRow.reason   (single writer)
AllocStatusRow.reason: Option<TransitionReason>  (rkyv-archived in ObservationStore)
  ├→ snapshot:  handlers::alloc_status reads AllocStatusRow.reason →
  │               last_transition.reason (byte-equal)
  └→ stream:    action_shim emits LifecycleEvent on broadcast →
                 streaming_submit_loop forwards →
                 SubmitEvent::LifecycleTransition.reason (byte-equal)
```

Drift between the two surfaces is structurally impossible because they read the same
field on the same row. KPI-04 (failure-reason coherence across surfaces) is asserted
as a property test over every `TransitionReason` variant in S-CP-07; KPI-02 closes the
loop end-to-end in S-WS-02.

## KPIs (outcome)

From `discuss/outcome-kpis.md` — five feature-level KPIs:

- **KPI-01** — Time to first NDJSON event ≤ 200 ms p95 on healthy local control plane.
  ✅ S-CP-02 asserts under `SimClock`-controlled DST (1024 cases); push-based
  broadcast-emit is structurally sub-tick latency. Real-time check on the Tier-3
  scenario `S-CLI-01` uses 200 ms, comfortably above CI runner jitter.
- **KPI-02** — Submit-with-bad-binary surfaces failure inline (boolean). ✅ S-WS-02
  asserts: real `tokio::process::Command::spawn` against `/usr/local/bin/no-such-binary`
  → ENOENT → cause-class `TransitionReason::ExecBinaryNotFound { path }` → exit 1
  with verbatim driver error in stdout AND in subsequent `alloc status` invocation's
  `error:` line, with the structured cause field byte-equal across surfaces. The
  load-bearing scenario for the entire feature.
- **KPI-03** — `alloc status` snapshot field count ≥ 6 actionable fields. ✅ S-AS-01
  enumerates state, resources, started_at, last_transition (from→to, reason, source),
  restart_budget; the journey TUI mockup matches verbatim.
- **KPI-04** — Failure-reason coherence across surfaces (boolean). ✅ S-CP-07 property
  test over all TransitionReason variants asserts streaming reason byte-equals
  snapshot reason for the same allocation; structurally enforced by [C6] above.
- **KPI-05** — `--detach` exits ≤ 200 ms p95 on healthy local control plane. ✅ S-CLI-01
  asserts wall-clock from invocation to exit; the streaming-default introduction does
  not regress `--detach` performance because `--detach` does not engage the NDJSON
  consumer at all.

All six ODI outcomes (1: time to know if spec converged; 2: silent-accept-while-failing;
3: time to identify failure reason; 4: effort to observe transitions without external
tools; 5: likelihood of re-deriving state from sparse output; 6: time to distinguish
"not yet" from "failed") have at least one defending KPI.

## Lessons learned

1. **Cause-class taxonomy beats verbatim text on the wire — but the design surface
   is bigger than it looks.** The original ADR-0032 / ADR-0033 shape carried the
   driver error string verbatim in a `reason: String` field. The DESIGN review
   surfaced two problems: (a) the CLI renderer would have to parse text it did not
   produce, and (b) the cross-surface byte-equality guarantee would be load-bearing
   on the action shim never normalising the string differently between writes (an
   easy bug to introduce). The same-day amendment to a structured `TransitionReason`
   enum + a verbatim `detail: Option<String>` slot for audit is the right shape, but
   it required: a 14-variant taxonomy, supporting types (`StoppedBy`, `CancelledBy`,
   `ResourceEnvelope`), the renderer mapping table, the action-shim classifier
   prefix matcher, the proptest `Arbitrary` impl, and a corresponding reshape of
   `TerminalReason` into structured-payload variants carrying `cause: TransitionReason`.
   This was a meaningful chunk of design surface that **only existed because we
   committed to single-source-of-truth across two consumption surfaces**. Future
   features that emit the same domain-event on multiple surfaces should reach for
   structured-cause-classification from the start.

2. **The action-shim classifier is the architectural answer to "where does string
   parsing live?"** The `DriverError::StartRejected.reason` text was already being
   discarded by the action shim's `Err(_) => AllocState::Terminated` arms. Step 02-01
   replaced those arms with a small prefix matcher inside `action_shim::dispatch_single`
   that classifies the text into a cause-class variant before writing the row. The
   classifier lives at the I/O boundary because the I/O boundary is the only place
   that has both (a) the verbatim driver text and (b) the wire-typed enum we want to
   write. Putting it in the reconciler would violate purity; putting it in the
   handler would force every consumer to re-classify on read. The classifier is one
   prefix-matcher function with an exhaustive switch and a `_ => Unknown { detail }`
   fallback; it is the right size for the right place.

3. **DST + property tests caught a structural bug the hand-written tests would have
   missed.** S-CP-07's property test (1024 cases over every `TransitionReason`
   variant, asserting streaming reason byte-equals snapshot reason) found a case
   where a mid-`reconcile` row write was producing a `LifecycleEvent` with one
   `TransitionReason` payload while the row in the ObservationStore carried a
   slightly different one — the action shim had two write sites and only one was
   wired through the classifier. A hand-written test on `ExecBinaryNotFound` alone
   would have passed because that path was correctly wired. The property test
   forced enumeration. The fix was a 1-line refactor to route both write sites
   through the same classifier helper. **Property tests are the right shape when
   the invariant is "this surface must equal that surface across every input shape."**

4. **`tokio::sync::broadcast`'s `Lagged` handling is non-trivial and worth
   designing in upfront.** The streaming-submit loop subscribes to `lifecycle_events`
   and forwards filtered events to NDJSON. A slow client (or a server transient)
   produces `RecvError::Lagged(n)`. The naive handling is "skip the lagged events";
   the correct handling per ADR-0032 §7 is "fall back to a one-shot
   `ObservationStore::alloc_status_rows()` snapshot, synthesise a single
   `LifecycleTransition` from the latest row, resubscribe, continue." Phase 1
   single-node single-subscriber makes lag unlikely, but the recovery path exists
   structurally for future multi-tenant cases. S-CP-10 asserts the recovery shape
   under DST.

5. **The walking-skeleton waiver was correct, and the driving-adapter mandate is
   the gate that matters anyway.** DWD-01 documented that the WS waiver is
   structural — the end-to-end already exists, the `submit` verb already commits,
   the lifecycle reconciler already converges, the `alloc status` endpoint already
   returns. There is no thinnest-vertical-slice to ship because the slice already
   ships. What ships in place of a formal WS is two Tier-3 scenarios (`S-WS-01`
   happy path, `S-WS-02` broken-binary regression target) carrying the conventional
   `@walking_skeleton @driving_adapter @real-io` tag triple so the catalogue audit
   picks them up. Quinn's mandate (every driving adapter has at least one Tier-3
   scenario) is the actual gate; the WS gate is a special case of it. **Brownfield
   features should waive WS structurally and lean on driving-adapter Tier-3
   coverage instead.**

6. **`std::io::IsTerminal` is in stdlib (Rust 1.70+) — no `atty` dep needed.** The
   CLI auto-detect uses `std::io::IsTerminal::is_terminal(&std::io::stdout())`
   directly. No `atty` crate (which has been deprecated upstream for several years
   anyway), no `isatty`-via-libc dependency. The trait seam in step 03-02 (a thin
   wrapper exposing `fn stdout_is_terminal() -> bool` for Tier-1 testability)
   keeps production wired to the stdlib and tests wired to a fake. This is the
   right shape; future CLI features that need TTY-aware behaviour should follow it.

7. **DELIVER-wave throughput is high when DESIGN+DISTILL pre-stage cross-cutting
   derives.** DWD-03 explicitly deferred 4 net-new types (`TransitionSource`,
   `TransitionRecord`, `SubmitEvent`, `LifecycleEvent`) from RED scaffold to crafter
   step 02-01 GREEN because they depend on cross-cutting derives (`DriverType:
   ToSchema`) that should land atomically with the slice's logic, not in advance.
   The crafter inherited a clean compile baseline (`cargo check -p overdrive-core
   -p overdrive-control-plane --tests` green on the scaffold commit) and could
   focus on logic, not on compile errors. **DISTILL waves should explicitly
   classify net-new types as RED-now or DELIVER-then based on cross-cutting
   derive dependencies; getting this right collapses the inner-loop friction in
   DELIVER.**

## Issues encountered

- **Step 03-02 PREPARE re-entered mid-step.** The execution log shows two PREPARE
  events for `03-02` — the first at 17:13:26Z (followed immediately by RED_ACCEPTANCE
  at 17:15:22Z and RED_UNIT skip at 17:15:25Z), then a second PREPARE at 17:31:33Z
  (followed by another RED_ACCEPTANCE/RED_UNIT pair at 17:31:48Z), and finally GREEN
  at 17:58:52Z and COMMIT at 18:05:36Z. The wave-decisions don't explain the
  re-entry directly; the most plausible reading is that the orchestrator/crafter
  noticed the original PREPARE produced the wrong scaffold shape (the IsTerminal
  trait seam abstraction wasn't right on the first pass — DWD-02 specifies "extract
  the IsTerminal probe behind a thin trait seam to keep S-CLI-02 Tier-1 testable
  without subprocess overhead"; the second PREPARE's `RED_UNIT.d` field reads
  "pure dispatch decision exercised through Tier-1 acceptance ... no internal class
  to unit-test" suggesting the trait seam moved between the two PREPAREs). No
  scaffold corruption; the GREEN that followed was clean and the COMMIT landed
  10 minutes later. Action: orchestrators should consider documenting mid-step
  PREPARE re-entries explicitly in the execution log's `d` field (currently the
  schema only carries them through repeated PREPARE entries), so the audit trail
  is complete.

- **No other escalations, scaffold rejections, or blocked steps.** Every step
  reached COMMIT/PASS on first attempt aside from the 03-02 re-entry above. This
  is consistent with the lessons-learned observation that DESIGN+DISTILL
  pre-staging (cross-cutting derive deferrals; cause-class taxonomy reshape
  before DELIVER opened) collapsed the DELIVER risk surface. The wave shipped 9
  steps in approximately 4.5 hours of wall-clock time on 2026-04-30 (first
  PREPARE 13:45:35Z → final COMMIT 18:05:36Z).

## Migrated artifacts

- **Architecture**: [`docs/architecture/cli-submit-vs-deploy-and-alloc-status/`](../architecture/cli-submit-vs-deploy-and-alloc-status/)
  — `architecture.md`, `c4-system-context.md`, `c4-container.md`, `c4-component.md`,
  `reuse-analysis.md`, `upstream-changes.md`.
- **Scenarios**: [`docs/scenarios/cli-submit-vs-deploy-and-alloc-status/`](../scenarios/cli-submit-vs-deploy-and-alloc-status/)
  — `test-scenarios.md` (26 scenarios, 16 happy-path / 10 error-path), `walking-skeleton.md`
  (the WS-waiver record + driving-adapter coverage).
- **UX journeys**: [`docs/ux/journeys/cli-submit-vs-deploy-and-alloc-status/`](../ux/journeys/cli-submit-vs-deploy-and-alloc-status/)
  — `journey-submit-streams-default.yaml` (the chosen direction; rewrites step 1 →
  1+1a+1b+1c, rewrites step 4), `journey-submit-streams-default-visual.md`.
- **ADRs**: ADR-0032 + ADR-0033 amendments live at their permanent home in
  `docs/product/architecture/` (no per-feature ADR migration directory exists in
  this workspace).

## What this unblocks

This feature closes the operator-trust gap on the inner-loop submit verb that prior
Phase 1 features structurally enabled but did not surface. The streaming-submit
machinery (broadcast bus, content-negotiation, cap timer, lagged-recovery, NDJSON
consumer) is reusable for any future multi-step operation that wants the same
shape — the obvious near-term candidates are `POST /v1/jobs/{id}:start`,
`POST /v1/jobs/{id}:restart`, and (when migration lands in Phase 3+)
`POST /v1/jobs/{id}:migrate`. The cause-class `TransitionReason` enum is also
reusable: any future driver class (microvm, wasm, unikernel) lands as new variants
in the same enum, and the action-shim classifier is the place to add the new
prefix-matchers.

For the operator: the broken-binary case the user originally hit now exits 1 with
the right answer. The feature shipped what it set out to ship.
