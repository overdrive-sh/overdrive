# Slice 04 — Service submit preserves `ConvergedRunning` semantics (regression guard)

**Outcome**: existing Service-shaped tests (long-running workloads, e.g. `/bin/sleep
3600`) continue to pass with vocabulary changed to "Service" but semantics unchanged.
No behavioural regression for the existing happy path.

**Stories**: US-04 (Service preservation).

**Learning hypothesis**: the kind discriminator does not break existing operator
workflows. By renaming "Job" → "Service" only in the rendered vocabulary (not in the
CLI verb `overdrive job submit` — that stays for a future feature) and preserving the
streaming protocol's `ConvergedRunning` semantics on the Service code path, existing
test fixtures continue to pass with minimal change.

## What ships in this slice

- The render function `format_running_summary` is reachable ONLY from the Service
  code path. Its rendered string changes from `Job '{name}' is running with N/M
  replicas (took {duration})` to `Service '{name}' is running with N/M replicas (took
  {duration})`.
- The literal `"live"` is REMOVED from the call site; the duration is computed from
  the injected `Clock` (RCA root cause D fix). This is the cosmetic-but-worthwhile
  fix from the RCA's Solution D.
- Existing integration test fixtures that use long-running binaries
  (`/bin/sleep 3600` etc.) are migrated to the `[service]` shape.
  Specifically: `crates/overdrive-cli/tests/integration/streaming_submit_happy_path.rs`
  and any other test that submits a Service-shaped spec.
- The CLI verb `overdrive job submit` is RETAINED in this feature; renaming it to
  `overdrive submit` (kind-agnostic) is a follow-up not included here.

## End-to-end value

- An operator who is using Overdrive today for long-running workloads continues to
  see the same flow: submit, stream events, get a "Service ... is running with N/M
  replicas (took 1.4s)" line. The vocabulary is more honest (says "Service") and the
  duration is real (no more `"live"` literal).

## Acceptance evidence

- All existing tests under `crates/overdrive-cli/tests/integration/` that exercise
  Service-shaped specs are migrated to the `[service]` shape and continue to pass.
- A new test asserts that the rendered duration is a measured value, not the literal
  `"live"` (paired with the grep gate from Slice 01).
- The honesty KPI for Service is unchanged at 100% (Service was already correct;
  this slice preserves that).

## Effort estimate (advisory)

~0.5 days. Mostly mechanical:
- Rename "Job" → "Service" in `format_running_summary`.
- Replace `"live"` with `clock.now() - submit_start_at` formatting.
- Migrate test fixtures.

## Risks

- The "Job" → "Service" rename in render output is operator-visible. Documentation
  that mentions the old phrasing must be updated. (Minor — Phase 1 docs are slim.)
- The `format_stopped_summary` function uses "Job" today (`Job '{name}' was stopped by
  {initiator}`). It also needs renaming to "Service" or kind-aware variants. Architect
  decides whether to fold this into Slice 04 or split.

## DoR fit

Regression guard. Without this slice, the rename in Slice 02 leaves Service-shaped
tests broken because they expect "Job" in the rendered string.
