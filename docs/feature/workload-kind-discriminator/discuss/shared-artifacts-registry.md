# Shared Artifacts Registry — workload-kind-discriminator

This file pins the single source of truth for every `${variable}` that flows across the
four journeys (A: submit Service, B: submit Job, C: submit Scheduled Job, D: alloc
status). Any artifact missing a documented source is a horizontal-integration risk and
must be remediated before DESIGN handoff.

## Changed Assumptions

- 2026-05-10 — folded in GH #164 (service listener spec shape). Two new
  artifacts added below: `${listener_triple}` and `${vip_assignment_state}`.
  Both flow across journey A (submit Service) and journey D (alloc status,
  Service sub-path) and must round-trip byte-identically.

## Registry

### `${spec_path}`

- **Source of truth**: operator filesystem (the path passed to `overdrive job submit`).
- **Consumers**: parser, submit RPC payload, error messages naming the file.
- **Owner**: operator.
- **Integration risk**: LOW — the value flows in one direction (operator → CLI →
  server) and is not stored.
- **Validation**: parser errors must echo the path the operator typed, byte-identical.

### `${kind}` — THE LOAD-BEARING ARTIFACT

- **Source of truth**: the parser's output enum (sketch:
  `WorkloadSpec::{Service(...) | Job(...) | Schedule(...)}`). Derived from section
  presence in the TOML.
- **Consumers** (full list — every consumer is a place this artifact must agree):
  1. CLI submit echo line (`kind=Service` / `kind=Job, run-to-completion` /
     `kind=Schedule`).
  2. Submit RPC variant (`SubmitService` / `SubmitJob` / `SubmitSchedule` — or one RPC
     with a kind discriminator on the wire).
  3. Streaming protocol event-type selection (`ServiceSubmitEvent`,
     `JobSubmitEvent`, `ScheduleSubmitEvent` — three enums per research R2; the bug-
     fix mechanism rests on these being separate types so the call site for
     `format_running_summary` does not exist on the Job code path).
  4. `AllocStatusRow.kind` — denormalised at write time from the originally-submitted
     spec.
  5. CLI render branch in `alloc status` (Service / Job / Schedule sub-paths).
  6. Error messages on rejected mixed-kind specs.
- **Owner**: parser at the spec module boundary (single Rust enum; no string-typed
  field).
- **Integration risk**: **HIGH** — every false positive in the bug under audit is a
  ${kind} disagreement between consumers. The structural fix is to make ${kind}'s
  domain a closed Rust enum at the parser boundary and have every downstream consumer
  match exhaustively on it.
- **Validation**: round-trip property — for any submitted spec with kind K, the
  corresponding `AllocStatusRow.kind` is K, and the streaming events emitted are
  variants of the kind-K event enum.

### `${spec_digest}`

- **Source of truth**: `ContentHash::of(rkyv::to_bytes(spec))` — same shape as today's
  Phase 1 walking skeleton.
- **Consumers**: submit echo (CLI), `AllocStatusRow.spec_digest`, alloc status render
  header, DescribeJob response.
- **Owner**: `overdrive-core` content-hash crate function.
- **Integration risk**: MEDIUM — a digest mismatch breaks the J-OPS-002 byte-identical
  round-trip guarantee. Existing Phase 1 tests cover this; extend them per kind.
- **Validation**: integration test asserts `submit_output.spec_digest ==
  alloc_status_output.spec_digest` for every kind.

### `${endpoint}`

- **Source of truth**: `ApiClient::base_url()` per `crates/overdrive-cli/CLAUDE.md` —
  read from the config at `~/.overdrive/config`.
- **Consumers**: submit echo, transport-error rendering.
- **Owner**: `overdrive-cli` config layer.
- **Integration risk**: LOW — already pinned by existing CLI conventions.

### `${exit_code}`

- **Source of truth**: `ExitObserver` (existing Phase 1 component at
  `crates/overdrive-control-plane/src/worker/exit_observer.rs`).
- **Consumers**: `JobSubmitEvent::{Succeeded, Failed}` payload, alloc status per-
  attempt Exit column, terminal verdict line.
- **Owner**: ExitObserver (no change to its interface).
- **Integration risk**: MEDIUM — the new feature introduces NEW consumers (Job-kind
  streaming events, kind-aware alloc status). The exit code MUST flow from kernel →
  ExitObserver → terminal-event payload → CLI render with no transformation.
- **Validation**: integration test asserts `kernel_exit_code == cli_displayed_exit_code`
  end-to-end.

### `${duration}`

- **Source of truth**: injected `Clock` (`SystemClock` in production, `SimClock` in
  tests) — never a literal sentinel.
- **Consumers**: streaming "took <duration>" line, alloc status per-attempt Duration
  column.
- **Owner**: callers that `clock.now()` at start and end of a measured interval.
- **Integration risk**: HIGH — RCA root cause D was the literal `"live"` masquerading
  as a duration. The new feature replaces every render-path duration with a measured
  value.
- **Validation**: no string literal `"live"` may appear in the production source after
  this feature. A grep gate (or `dst-lint`-style scanner) enforces this.

### `${attempt_count}` and `${max_attempts}`

- **Source of truth**: Job-lifecycle reconciler's restart-attempt history rows;
  `RESTART_BACKOFF_CEILING` const at `crates/overdrive-core/src/reconciler.rs:RESTART_BACKOFF_CEILING`
  (currently 5 in Phase 1; the Job kind may want this overridable per-spec via
  `backoff_limit` per research R1).
- **Consumers**: streaming intermediate "attempt N/M" line, alloc status per-attempt
  table.
- **Owner**: reconciler.
- **Integration risk**: MEDIUM — count agreement across CLI surfaces is necessary for
  operator trust.
- **Validation**: integration test that submits a Job with `backoff_limit = 3` asserts
  the streaming intermediate lines and alloc status per-attempt table both name "of 3".

### `${verdict}`

- **Source of truth**: derived in render layer from `(AllocStatusRow.state,
  reconciler-side BackoffExhausted claim, kind-aware predicate)`.
- **Domain**: `{Succeeded, Failed, Failed (backoff exhausted), In progress, Unknown}`
  for Job kind.
- **Consumers**: alloc status header line for Job kind.
- **Owner**: render layer pure function.
- **Integration risk**: MEDIUM — verdict must agree with the streaming protocol's
  terminal event for the same alloc.
- **Validation**: cross-surface property — for any Job that has reached terminal,
  `cli_streaming_terminal_event ↔ alloc_status_verdict` must agree.

### `${cron_expr}`

- **Source of truth**: `[schedule].cron` in the originally-submitted TOML.
- **Consumers**: alloc status render for Schedule kind.
- **Owner**: parser.
- **Integration risk**: LOW (this slice) — execution semantics that consume the cron
  expression are deferred; this slice ships parser + display only.
- **Validation**: alloc status renders the byte-identical string the operator wrote.

### `${deferral_issue_url}`

- **Source of truth**: a single CLI config constant (e.g.
  `crates/overdrive-cli/src/render/deferrals.rs::SCHEDULE_EXECUTION_TRACKING_URL`).
- **Consumers**: submit echo NOTE block (Schedule kind), alloc status Reason line
  (Schedule kind).
- **Owner**: CLI render layer. Value is fixed at
  `https://github.com/overdrive-sh/overdrive/issues/166` (deferral approved by user
  2026-05-09; tracking issue created the same day).
- **Integration risk**: HIGH — if the constant is hardcoded in two places they will
  drift; a single SSOT is mandatory.
- **Validation**: `assert_eq!(submit_echo.deferral_url, alloc_status.deferral_url)` in
  the integration test.

### `${listener_triple}` — Service listener spec shape (folded in 2026-05-10)

- **Source of truth**: parser output — `Vec<Listener>` field on
  `WorkloadSpec::Service`, where `Listener { port: NonZeroU16, protocol:
  Proto, vip: Option<ServiceVip> }`. Reuses the existing
  `overdrive-core::Proto` newtype; the spec layer does not define a second
  copy.
- **Consumers**:
  1. CLI submit echo `Listeners:` section (Service kind only).
  2. CLI `alloc status --job <id>` `Listeners:` section (Service render
     branch only).
  3. `AllocStatusRow` listener fields denormalised at write time (architect
     to confirm shape).
  4. Parser uniqueness validation: no two listeners within a Service may
     share `(vip, port, protocol)`.
- **Owner**: parser at the spec module boundary.
- **Integration risk**: HIGH — KPI K6 asserts byte-equality between submit
  echo and `alloc status` Listeners sections across 100 Service spec submits
  with pinned VIPs. Drift between the two render paths breaks K6.
- **Validation**: round-trip property test asserts `JobSpecInput` ↔ `Job` ↔
  TOML/JSON preserves listener order and triple values bit-equivalently.

### `${vip_assignment_state}` — pinned IPv4 vs. pending allocation

- **Source of truth**: parser output — `Option<ServiceVip>` per listener.
  `Some(addr)` means the operator pinned a VIP; `None` means the platform
  must allocate (per #167) or reject at admission (also per #167's eventual
  decision). The spec layer is forward-compatible with both runtime
  outcomes.
- **Domain**: pinned-IPv4 string (e.g. `"10.0.0.1"`) OR the literal string
  `(vip: pending allocation — see #167)`.
- **Consumers**:
  1. CLI submit echo per-listener line.
  2. CLI `alloc status` per-listener line (Service render branch).
- **Owner**: parser; render layer applies the `(vip: pending allocation —
  see #167)` literal when the field is `None`.
- **Integration risk**: HIGH — the pending-VIP marker is the first place a
  reader of either submit echo or `alloc status` sees the deferral
  reference. It must be byte-identical across the two surfaces; the
  literal must also reference the canonical issue URL #167. Drift between
  the two surfaces breaks operator trust ("did I get the VIP or didn't I?").
- **Validation**: integration test asserts the literal equals
  `(vip: pending allocation — see #167)` byte-for-byte; the trailing issue
  number is sourced from a single CLI config constant
  (`SERVICE_VIP_ALLOCATOR_TRACKING_URL` or similar — architect to name).

## Validation gates

### Consistency check

Run for every artifact above:

1. ✅ Source of truth exists in source code (or named placeholder for the deferral URL
   constant).
2. ✅ Every consumer references the source rather than hardcoding.
3. ✅ No artifact is missing a documented source.
4. ✅ No two steps in the journeys display the same data from different sources.

### Cross-journey property tests (DISTILL wave handoff)

The DISTILL wave (acceptance-designer) must cover:

- For every kind K, every `AllocStatusRow` written for a K-kind spec carries
  `kind == K`.
- For every Job-kind submit, the streaming terminal event's exit_code equals the
  alloc status's per-attempt Exit value for the same attempt.
- The deferral URL is byte-identical across the submit echo and alloc status outputs.
- The string `"live"` never appears as a render literal in production source after
  this feature lands.
- For every Service submit with N listeners, `alloc status` emits a Listeners
  section with N lines, each line byte-identical to the submit echo's
  corresponding listener line. (KPI K6.)
- The pending-VIP marker `(vip: pending allocation — see #167)` is sourced
  from a single CLI config constant; submit echo and `alloc status` cannot
  drift.

## Quality gates

- **Journey completeness**: ✅ all four journeys have steps, commands, emotional
  annotations, shared artifacts, integration checkpoints.
- **Emotional coherence**: ✅ each journey's arc is internally smooth; transitions
  named explicitly.
- **Horizontal integration**: ✅ every artifact above has a single source of truth
  and a documented consumer list.
- **CLI UX compliance**: ✅ command structure (`overdrive job submit`,
  `overdrive alloc status --job <id>`) consistent across journeys; no new verbs
  introduced.
