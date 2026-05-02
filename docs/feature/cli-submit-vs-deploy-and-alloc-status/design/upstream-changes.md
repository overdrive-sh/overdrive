# Back-prop required after DESIGN re-open (2026-04-30)

**Wave**: DESIGN amendment
**Trigger**: `TransitionReason` refactored from state-class to cause-class.
**Authoritative ADRs**: ADR-0032 §3 (Amendment 2026-04-30) + ADR-0033 §4
(Amendment 2026-04-30).
**Scaffold**: `crates/overdrive-core/src/transition_reason.rs` (rewritten);
`crates/overdrive-control-plane/src/api.rs::TerminalReason` (extended).

This list is **not edits already made**. It catalogues edits that the
product-owner (DISCUSS), acceptance-designer (DISTILL), and software-
crafter (DELIVER pre-execution) agents must apply on their next pass.
DESIGN deliberately does not edit DISCUSS / DISTILL / roadmap
artifacts.

---

## To DISCUSS (`nw-product-owner`)

### File: `docs/feature/cli-submit-vs-deploy-and-alloc-status/discuss/shared-artifacts-registry.md`

- **Section: `### convergence_event — typed NDJSON line`**
  Lock the cause-class `TransitionReason` variant list. The current
  text ("Variants (DESIGN names; this is the journey-level shape)")
  deliberately deferred variants to DESIGN; DESIGN has now committed
  the full Phase 1 variant set. Replace the deferral with the locked
  shape: progress markers (`Scheduling`, `Starting`, `Started`,
  `BackoffPending { attempt }`, `Stopped { by }`) + cause-class
  failure variants (`ExecBinaryNotFound { path }`,
  `ExecPermissionDenied { path }`, `ExecBinaryInvalid { path, kind }`,
  `CgroupSetupFailed { kind, source }`, `DriverInternalError { detail }`,
  `RestartBudgetExhausted { attempts, last_cause_summary }`,
  `Cancelled { by }`, `NoCapacity { requested, free }`) + Phase 2
  emit-deferred (`OutOfMemory { peak_bytes, limit_bytes }`,
  `WorkloadCrashedImmediately { exit_code, signal, stderr_tail }`).
  Cite ADR-0032 §3 Amendment 2026-04-30 as the SoT.

- **Section: `## Cross-cutting: transition_reason`**
  Update the "One source / Two consumers" paragraph. The opaque-string
  language ("e.g. `scheduling on local`, `backoff_exhausted`") is now
  stale: cause-class variants carry typed payloads, not free-form
  strings. The reconciler emits typed cause variants (`NoCapacity`,
  `RestartBudgetExhausted`); the action shim classifies
  `DriverError::StartRejected.reason` text into the right cause-class
  variant via a small string-prefix matcher (catalogued in ADR-0032
  §4). The verbatim driver text remains preserved in
  `AllocStatusRow.detail` for audit, but is no longer the primary
  cause-carrier.

### File: `docs/feature/cli-submit-vs-deploy-and-alloc-status/discuss/user-stories.md`

- **Story US-02 — Domain Examples #1 (Binary not found — ENOENT)**
  The example references the wire shape of `ConvergedFailed`. Update
  the structured-cause to reflect the cause-class shape: instead of
  `"reason: driver start failed (binary not found)"` (rendering the
  old `DriverStartFailed` unit variant + `detail`), the AC should
  render `"reason: binary not found: /usr/local/bin/payments"`
  (cause-class `ExecBinaryNotFound { path }` direct rendering). The
  verbatim driver error on `last-event:` line stays.
  Reference: ADR-0033 §4 amended mapping table.

- **Story US-02 — Acceptance Criteria**
  AC #5 ("The `reason` string in `ConvergedFailed` for a given
  allocation equals the `last_transition.reason` rendered by `alloc
  status` for the same allocation, byte-for-byte") — strengthen to
  call out byte-equality of the **typed payload**, not just the
  variant tag. The structured `data: { path: "..." }` portion must
  match across surfaces too. Reference: ADR-0033 §5 amended.

- **Story US-05 — Domain Example #2 (Failed allocation post-broken-submit)**
  The example shows `reason: driver start failed`. Update to
  `reason: binary not found: /usr/local/bin/payments` to match the
  cause-class rendering from ADR-0033 §4 amended mapping table. The
  verbatim `error:` line on the next row stays as-is (it carries the
  raw `errno`-decorated text from `std::io::Error::Display`).

- **Story US-06 — Domain Example #1 (Same string in both surfaces — broken binary)**
  The streaming and snapshot byte-equality assertion now covers the
  typed payload. Add explicit language: "the `data` object inside
  the `kind: exec_binary_not_found` reason serialises identically
  across both surfaces, including the `path` field." This was
  implicit before; cause-class makes it explicit.

- **Story US-06 — Domain Example #2 (Same string — backoff exhaustion)**
  Update reference: streaming `ConvergedFailed { terminal_reason:
  BackoffExhausted { attempts: 5, cause: ExecBinaryNotFound { path } } }`
  (the new `TerminalReason::BackoffExhausted` carries `attempts` AND
  the cause-class cause variant of the final failed attempt). Snapshot
  shows `Restart budget: 5 / 5 used (backoff exhausted)` AND the
  per-row `last_transition.reason` carries the cause variant
  (`ExecBinaryNotFound { path }`). The byte-equality applies to the
  cause variant on both surfaces.

- **Story US-06 — Domain Example #3 (Same string — server timeout)**
  Update reference: streaming `ConvergedFailed { terminal_reason:
  Timeout { after_seconds: 60 } }`. The wire shape now carries the
  cap value, not a free-form `error: "did not converge in 60s"` string
  — the CLI renders the prose from the structured field.

---

## To DISTILL (`nw-acceptance-designer`)

### File: `docs/feature/cli-submit-vs-deploy-and-alloc-status/distill/test-scenarios.md`

- **Scenario S-CP-05 (Driver-start-failed transition surfaces verbatim driver text in detail)**
  Rename + retarget. The scenario currently asserts
  `reason: driver_start_failed` and `detail: "stat /no/such: ..."`.
  Replace with cause-class assertion:
  - sim driver returns `DriverError::StartRejected { reason: "spawn /no/such: No such file or directory (os error 2)" }`
  - action shim's classifier produces
    `TransitionReason::ExecBinaryNotFound { path: "/no/such" }`
  - NDJSON line carries `reason: { kind: "exec_binary_not_found", data: { path: "/no/such" } }`
  - `detail` field carries the verbatim text (preserved for audit)
  Add scenarios for the other Phase 1 cause-class classifications:
  ENOENT → `ExecBinaryNotFound`; EACCES → `ExecPermissionDenied`;
  ENOEXEC → `ExecBinaryInvalid`; cgroup-failure → `CgroupSetupFailed`;
  unclassified → `DriverInternalError`. One scenario per branch is
  sufficient; the property-test shape covers the rest. Reference:
  ADR-0032 §4 amended classification table.

- **Scenario S-CP-06 (Server wall-clock cap fires)**
  Update terminal-line shape: now
  `ConvergedFailed { terminal_reason: TerminalReason::Timeout { after_seconds: 60 } }`
  not the previous unit-variant shape. The error-string assertion
  (`"did not converge in 60s"`) becomes a CLI-render assertion derived
  from `after_seconds`, not a wire-field assertion.

- **Scenario S-CP-07 (Streaming reason and snapshot reason are the same TransitionReason value, KPI-04, T1 mirror)**
  The "8 (exhaustive)" property-test cardinality is wrong. The cause-
  class enum has 14 Phase 1 variants (5 progress markers + 9 cause-
  class) plus 2 Phase 2 emit-deferred for forward-compat (16 total).
  Update the Property-test shape to: parametrise over a representative
  subset of variants (the 14 Phase 1 emit) + structurally-distinct
  payloads (e.g. for `ExecBinaryNotFound`, generate path values). The
  S-CP-07 generator becomes a richer proptest. Reference: ADR-0032
  §3 amended variant list.

- **Scenario S-CP-09 (AllocStatusRow round-trips through rkyv with the new reason and detail fields)**
  Update Property-test shape: was "every TransitionReason × Option<String> × every AllocState".
  Now: every TransitionReason variant (with proptest-generated
  payloads for cause-class variants) × Option<String> × every
  AllocState. The proptest must construct each variant via its
  `Arbitrary` impl (the crafter writes one alongside the type). 1024
  cases stays; the per-case work grows but stays in budget.

- **Scenario S-AS-02 (TransitionRecord and SubmitEvent::LifecycleTransition share the same TransitionReason enum)**
  No edit required. The compile-time type-equality assertion still
  holds — the same enum is used on both surfaces; only its variant
  set has grown. Strengthen the inline note: "byte-equality across
  surfaces extends to the cause-class typed payload (`data: { path:
  ... }` etc.), not just the variant tag."

- **Scenario S-AS-05 (CLI renders the Failed TUI mockup with verbatim driver error and `(backoff exhausted)`)**
  Update the rendering assertion. `Last transition: ... Pending →
  Failed reason: driver start failed source: driver(exec)` becomes
  `Last transition: ... Pending → Failed reason: binary not found: /usr/local/bin/payments source: driver(exec)`.
  The verbatim `error:` line stays. Reference: ADR-0033 §4 amended
  mapping table.

- **Scenario S-AS-07 (alloc_status handler projects AllocStatusRow.reason to AllocStatusRowBody.last_transition.reason byte-identically)**
  Update Property-test shape: was "every TransitionReason × Option<String>".
  Now: every TransitionReason variant (with proptest-generated
  payloads). The cardinality grows from 8 × N to ~14 × N (plus
  payload generation); 1024 cases stays.

- **Scenario S-WS-02 (Operator submits a broken-binary spec — REGRESSION TARGET)**
  No structural edit required, but tighten the byte-equality assertion:
  "the streaming `LifecycleTransition.reason` (the last
  `exec_binary_not_found` event) byte-equals the snapshot's
  `last_transition.reason` — both wire forms include the structured
  `data: { path: ... }` payload, not just the `kind` discriminator."
  This catches drift in the typed payload separately from the variant
  tag.

- **Section: `## 5. Property-shape scenarios summary`**
  Update the cardinality table:
  - `S-CP-07`: was "8 (exhaustive)" → now "14 + payload generation per cause-class variant"
  - `S-CP-09`: was "every TransitionReason × Option<String> × every AllocState" → now "every TransitionReason variant (cause-class with proptest payloads) × Option<String> × every AllocState"
  - `S-AS-07`: was "every TransitionReason × Option<String>" → now "every TransitionReason variant (cause-class with proptest payloads) × Option<String>"

- **Section: `## 1. Coverage map (story → scenario)`**
  No row edits required (the variant change is below the AC level
  the table indexes). Add a footnote: "All scenarios referencing
  `TransitionReason` byte-equality cover the cause-class typed
  payload, not just the `kind` discriminator (ADR-0032 §3 amendment
  2026-04-30)."

---

## To roadmap (`nw-software-crafter`, pre-execution)

### File: `docs/feature/cli-submit-vs-deploy-and-alloc-status/deliver/roadmap.json`

- **Step 01-01 (Land AllocState::Failed + AllocStatusRow.{reason,detail})**
  No structural edit. The criteria already names `TransitionReason`
  abstractly; the cause-class refactor lands inside the same step.
  Add to criteria text: "the `TransitionReason` enum is the cause-
  class shape per ADR-0032 §3 Amendment 2026-04-30 — 14 Phase 1
  variants + 2 Phase 2 emit-deferred." The S-CP-09 acceptance
  scenario remains the per-step gate; scope of the proptest grows
  per the DISTILL update above.

- **Step 01-02 (Promote api.rs RED scaffolds to GREEN: TransitionRecord, RestartBudget, ResourcesBody, AllocStateWire)**
  Update criteria: `TerminalReason` (already in `api.rs`) was
  extended in the DESIGN amendment to carry structured payloads
  (`BackoffExhausted { attempts, cause }`, `DriverError { cause }`,
  `Timeout { after_seconds }`). The 01-02 promotion still applies
  unchanged (the type is already declared; the step promotes it from
  scaffold to GREEN). Add to the Files list: the
  `TransitionReason` cause-class enum lands in 01-01, so the
  references in 01-02's new `TransitionSource`/`TransitionRecord`
  declarations resolve cleanly.

- **Step 01-03 (Extend AllocStatusResponse + rewrite alloc_status handler hydration + CLI snapshot renderer)**
  Update criteria text: the CLI render contract reference is now
  "ADR-0033 §4 amended mapping table" (cause-class variants render
  per the new table). The S-AS-07 property test count grows per the
  DISTILL update above. The acceptance scenario list is unchanged.

- **Step 02-01 (Land LifecycleEvent (broadcast payload) + broadcast::Sender on AppState + action shim emit)**
  Add to criteria: "the action shim's classification of
  `DriverError::StartRejected.reason` text into a cause-class
  `TransitionReason` variant (per ADR-0032 §4 amended classification
  table) lands in this step. The classifier is a small prefix matcher
  inside `action_shim::dispatch_single`; the verbatim text continues
  to populate `AllocStatusRow.detail` for audit." This is a per-
  variant code addition; existing `Err(DriverError::StartRejected
  { reason: _, .. }) => AllocState::Terminated` pattern at L117 / L148
  becomes
  `Err(DriverError::StartRejected { reason, driver }) => { let cause = classify_driver_failure(&reason, driver, &spec.command); /* write reason+detail+state */ }`.

- **Step 02-02 (Land SubmitEvent enum + TerminalReason wire serialization)**
  Update criteria: `TerminalReason` now uses the
  `#[serde(tag = "kind", content = "data", rename_all = "snake_case")]`
  shape (matches `TransitionReason`) — the variants are no longer
  `Copy` (they carry inner `cause: TransitionReason` payloads). The
  acceptance suite's serde round-trip proptest must construct
  representative payloads for each terminal variant, not just match
  on the discriminator.

- **Step 02-03 (Wire content negotiation in submit_job + streaming_submit_loop)**
  Update criteria: terminal-event detection now constructs the
  cause-class shape:
  - `state == Running && replicas_running >= replicas_desired` → `ConvergedRunning { alloc_id, started_at }` (unchanged)
  - `state == Failed && restart_budget.exhausted` → `ConvergedFailed { terminal_reason: TerminalReason::BackoffExhausted { attempts, cause: <last cause-class TransitionReason from row> } }`
  - cap timer fires → `ConvergedFailed { terminal_reason: TerminalReason::Timeout { after_seconds: cap.as_secs() as u32 } }`
  The "last cause-class TransitionReason from row" is read off the
  most recent `AllocStatusRow.reason` field for the alloc — same
  source the snapshot uses, structural single-source-of-truth pin
  preserved.

- **Step 02-04 (CLI NDJSON consumer + exit-code mapping + structured Error block + broken-binary regression target)**
  Update criteria: the `Error:` block renders the cause-class
  `TransitionReason` via `human_readable()` (e.g. `reason: binary
  not found: /usr/local/bin/payments`). The `last-event:` line stays
  on the verbatim `detail` text. The `Hint:` line is rendered
  generically when the variant is one of the cause-class diagnostic
  classes; the existing `"fix the spec's exec.command path and re-
  run"` hint maps to `ExecBinaryNotFound` and `ExecPermissionDenied`,
  while other cause variants get neutral hints (added to the CLI
  rendering crate per the same step). S-WS-02 (regression target)
  byte-equality assertion now covers the typed payload — see
  DISTILL S-WS-02 update above.

### Roadmap shape

The 9-step structure (Slice 01: 3 steps; Slice 02: 4 steps; Slice 03:
2 steps) **still holds**. The cause-class refactor changes the variant
list inside step 01-01 (the foundational write of `reason` to the
row) and ripples consequent text updates through steps 01-02, 01-03,
02-01, 02-02, 02-03, 02-04. No step is added, no step is dropped,
no step's dependencies change. Per-step time estimates do not need
revision (the per-variant work is small; the proptest cardinality
growth is within the existing per-step budget).

---

## Reuse Analysis re-evaluation

The original 14 EXTEND / 8 REUSE / 1 REPLACE / 8 CREATE NEW table
in `docs/feature/cli-submit-vs-deploy-and-alloc-status/design/reuse-analysis.md`
holds with **two CREATE NEW additions** the refactor introduces:

- `StoppedBy` enum (new, in `overdrive-core`) — replaces the unit
  variant `Stopped` with a payload-bearing one. Justification: the
  `Stopped` rendering distinguishes operator-driven stop from
  reconciler-converged stop; a String would invite drift; a typed
  enum closes the set.
- `CancelledBy` enum (new, in `overdrive-core`) — same shape as
  `StoppedBy` but for `Cancelled`. Justification: same as above;
  Phase 2 cluster-driven cancellation shares the wire shape.
- `ResourceEnvelope` struct (new, in `overdrive-core`) — carried by
  `NoCapacity { requested, free }`. Justification: mirrors
  `traits::driver::Resources` but kept in `transition_reason.rs` so
  the cause-class enum is self-contained at the wire boundary. The
  alternative (depending on `traits::driver::Resources` from inside
  the `TransitionReason` declaration) couples the wire-typed reason
  to the driver trait surface unnecessarily.

Updated counts:
| Disposition | Count |
|---|---|
| EXTEND | 14 |
| REUSE unchanged | 8 |
| REPLACE in place | 1 |
| CREATE NEW (with justification) | 11 (+3 from the cause-class refactor) |

The `nw-product-owner` updates `reuse-analysis.md` in lockstep with
the `shared-artifacts-registry.md` update above — both files are
DISCUSS / DESIGN cross-cutting and travel together.

---

## Compile-cleanliness during the GREEN transition

The DESIGN amendment is structurally complete in the codebase:

- `crates/overdrive-core/src/transition_reason.rs` — rewritten with
  the cause-class enum, `StoppedBy`, `CancelledBy`,
  `ResourceEnvelope`. `human_readable()` and `is_failure()` panic
  with the RED scaffold marker; the type itself compiles.
- `crates/overdrive-control-plane/src/api.rs::TerminalReason` —
  extended with structured payloads referencing
  `overdrive_core::TransitionReason`.
- `crates/overdrive-core/src/lib.rs` — `pub use
  transition_reason::TransitionReason` is unchanged; future re-
  exports for `StoppedBy` / `CancelledBy` / `ResourceEnvelope` land
  in slice 01-01 as part of the "GREEN transition_reason.rs"
  promotion.

The crafter's slice 01-01 GREEN step replaces the `panic!` bodies
with the rendering / classification logic from ADR-0033 §4 amended
mapping table. No new BROKEN compile state introduced by this
amendment.
