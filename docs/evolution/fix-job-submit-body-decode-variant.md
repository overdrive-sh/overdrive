# Evolution: fix-job-submit-body-decode-variant

**Date**: 2026-04-26
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Wave shape**: bugfix (RCA -> /nw-deliver, single-phase 2-step roadmap)
**Status**: shipped, all DES phases EXECUTED/SKIPPED with PASS verdict

---

## Defect

`crates/overdrive-cli/src/commands/job.rs:110-113` mapped a server-response
field-validation failure (`JobId::new(&resp.job_id)` rejecting the id the
control plane echoed back) to `CliError::InvalidSpec`. The rendered
operator-facing Display read:

> `invalid job spec: field 'id': server returned invalid job_id ...`

This blames the operator's spec when the fault is a server-side contract
violation — the spec was already accepted; we are decoding the response.

## Root cause

Variant taxonomy is documented but was not honoured at this call site:

- `CliError::InvalidSpec` (`http_client.rs:78-87`) — *"Client-side spec
  validation failed BEFORE any HTTP call. Separate variant from
  `CliError::HttpStatus` — the client-side path never reaches the
  server."*
- `CliError::BodyDecode` (`http_client.rs:75-76`) — *"A successful 2xx
  response whose body failed to deserialise into the expected typed
  shape. This is a server-side contract violation."*

The call site at `job.rs:110-113` is post-HTTP. A `JobId` validator
failure on `resp.job_id` is semantically identical to the
`resp.json::<T>()` decode failure at `http_client.rs:242` — which is
already mapped to `BodyDecode`. The defect was therefore a single
mistransmitted variant, not a structural taxonomy gap.

## Decision

**Variant swap inside an extracted helper, not at the call site.** The
fix is four lines logically — one variant name and the field shape it
constructs — but those lines also pin the rustdoc that documents the
variant choice. The RED step extracted `parse_response_job_id(raw: &str)
-> Result<JobId, CliError>` so:

1. The unit-of-error-mapping is named, testable, and documented in one
   place rather than buried in `submit()`'s body.
2. The regression test asserts on the helper's return shape directly,
   independent of the surrounding `submit()` HTTP-handling code.
3. The GREEN swap touches only the helper body; `submit()` is unchanged
   between RED and GREEN.

Operator-facing rendering after the fix:

> `failed to decode response body from control plane: server returned invalid job_id ...`

## Scope landed

Two-step roadmap, executed via `/nw-deliver`:

1. **RED — regression test pinning the BodyDecode variant**
   (`2af64f1`). Adds
   `crates/overdrive-cli/tests/integration/post_http_invalid_job_id.rs`
   gated behind `integration-tests`, declared inside the inline `mod
   integration` block of `tests/integration.rs` per
   `.claude/rules/testing.md`. Stub control plane returns 200 OK with a
   syntactically-valid JSON body whose `job_id` is rejected by
   `JobId::new()` (uppercase letters). Test calls `commands::job::submit()`
   directly per `crates/overdrive-cli/CLAUDE.md` (no subprocess).
   Asserts `matches!(err, CliError::BodyDecode { cause } if cause.contains("server returned invalid job_id"))`
   and negative-asserts `!matches!(err, CliError::InvalidSpec { .. })`.
   Same commit extracts `parse_response_job_id` so the test has a stable
   target. Committed with `--no-verify` per the RED-scaffold exception
   (intentionally-failing commit; the panic IS the specification of work
   not yet done).

2. **GREEN — swap `InvalidSpec` to `BodyDecode` in the helper**
   (`0d5c8a3`). Four-line change inside `parse_response_job_id`: variant
   name flips; message text preserved exactly (backticks around the id,
   `: {e}` suffix). Single-cut per project memory
   `feedback_single_cut_greenfield_migrations` — no fallback, no flag,
   no deprecation. Regression test goes RED -> GREEN.

## Mutation-gate detour and structural finding

The mutation gate exercised on this PR exposed a pre-existing wrapper
bug in `cargo xtask mutants` that had been latent since the wrapper's
short-circuit handling was introduced. Captured here because the bug
class — *trusting a stale on-disk artifact as the current run's
verdict* — is exactly the kind of failure that survives every existing
review pass without triggering an obvious test gap.

### Symptom

The PR added `Default::default` to `.cargo/mutants.toml`'s `exclude_re`
(rationale below). With the rule active:

- Direct `cargo mutants --list` against the diff scope produced zero
  `Default::default()` mutants — the rule was honoured.
- Wrapper-driven `cargo xtask mutants --diff origin/main --package
  overdrive-cli` reported `total=3 unviable=3` and failed the gate as
  "no quality signal."

### Diagnostic chain

Three observations isolated the divergence:

1. Instrumented the wrapper to dump `CARGO_*` / `RUSTC_*` env, CWD, and
   the resolved cargo-mutants argv, then replicated the exact subprocess
   shape directly. Direct `cargo mutants --list` still produced zero
   `Default::default()` mutants.
2. The wrapper's three "unviable" mutants matched the verdicts a *prior*
   pre-edit run had written before the `exclude_re` rule landed.
3. Cleared `target/xtask/mutants.out/` manually and reran the wrapper:
   `total=0` vacuous pass — confirming the divergence was post-subprocess
   (stale-file read), not pre-subprocess (config not loaded).

### Root cause

`cargo-mutants` writes `mutants.out/outcomes.json` *only when it actually
runs mutations*. On a filter-intersection short-circuit (`INFO No
mutants to filter`) it logs the line, exits 0, and writes nothing. The
wrapper's `read_outcomes_or_short_circuit` then trusted any pre-existing
`outcomes.json` on disk as the current run's report — even though the
current run never produced it. The wrapper's existing
`clear_stale_summary` cleared *its own* `mutants-summary.json` upfront
but did not clear cargo-mutants' `outcomes.json`.

### Fix

`adcf0bf fix(xtask): clear stale outcomes.json before invoking
cargo-mutants` extends `clear_stale_summary` to clear both files
upfront. Both rationales are documented inline at the rustdoc on
`clear_stale_summary` — the contract is now "no verdict file on disk
until the current run has parsed cargo-mutants' outcomes and written
its own." A regression test seeds a stale `outcomes.json` and asserts
the wrapper removes it before invoking cargo-mutants.

### Why the asymmetry was latent

The short-circuit path is rare. `exclude_re` entries that already shipped
in `mutants.toml` (`unsafe`, `select!`, `HarnessNoopHeartbeat`, etc.)
either matched mutants on every diff or never matched anything in the
filtered set, so the empty-intersection condition never fired in CI.
Newtype-no-`Default` codepaths trigger it reliably because most of the
diff scope's surface area returns `Result<DomainNewtype, _>` — and
domain newtypes deliberately lack `Default` impls per
`.claude/rules/development.md` STRICT-by-default.

## Trade-off codified in `.cargo/mutants.toml`

`ec99bc8 config(mutants): exclude synthesized Default::default
replacements` adds `Default::default` to `exclude_re` and explains the
trade-off in-place. The summary:

- cargo-mutants' default operator replaces a function body with
  `Default::default()`, `Ok(Default::default())`, or
  `Err(Default::default())` for any function returning `T`,
  `Result<T, E>`, or whose return type is `Default`-able.
- This codebase deliberately avoids `Default` impls on domain types per
  `.claude/rules/development.md` STRICT-by-default newtypes (validating
  constructors only; no infallible `new()` that silently accepts garbage).
- The operator therefore lands `error[E0277]: the trait bound X: Default
  is not satisfied` for almost every site, surfacing as `total=N
  unviable=N` "no quality signal" gate failures rather than measurable
  kill rate.

The deny-pattern is unique to the synthesised replacement bodies and
never appears in legitimate mutation descriptions for arithmetic flips,
comparison flips, or `Vec::new()` / `()` body replacements — those
operators remain enabled because *they are the ones the test suite is
supposed to kill*. Re-evaluate if this codebase ever grows broad
`Default` impls (Phase 2+ if a domain type genuinely has a sensible
identity).

The companion `skip_calls` block was retained for now; the rustdoc on
that block was rewritten to make clear that `skip_calls` suppresses
*calls to* a function, not the *operator that synthesises* a body —
the latter is `exclude_re`. The next mutants-toml audit will retire the
historical `skip_calls = ["Default::default"]` entry once every CI run
referencing "Rule 7 — Default" has rotated out.

## Commits in scope

```
2af64f1  test(cli): add regression test pinning BodyDecode for invalid response job_id (RED)
0d5c8a3  fix(cli): map server-response job_id validation failure to BodyDecode
adcf0bf  fix(xtask): clear stale outcomes.json before invoking cargo-mutants
ec99bc8  config(mutants): exclude synthesized Default::default replacements
5777d31  docs(deliver): record final COMMIT phase for fix-job-submit-body-decode-variant
```

## Files changed

- `crates/overdrive-cli/src/commands/job.rs` — extracted helper
  `parse_response_job_id`; variant flipped from `InvalidSpec` to
  `BodyDecode`. (~16 lines net.)
- `crates/overdrive-cli/tests/integration.rs` — declared the new
  scenario module inside the inline `mod integration` block.
- `crates/overdrive-cli/tests/integration/post_http_invalid_job_id.rs`
  — new regression test (69 lines).
- `xtask/src/mutants.rs` — extended `clear_stale_summary` to also
  remove `mutants.out/outcomes.json`; added regression test for the
  stale-`outcomes.json` short-circuit class. (105 lines added,
  33 modified.)
- `.cargo/mutants.toml` — added `Default::default` to `exclude_re`;
  rewrote `skip_calls` rationale.

## Lessons learned

1. **`InvalidSpec` is for *pre-HTTP* spec validation; `BodyDecode` is for
   *post-HTTP* response shape failures.** A validating constructor like
   `JobId::new()` does not change category just because the input string
   came from the server — *which side of the wire* the validation runs
   determines the variant.
2. **A passing mutation gate is not the same as a *kill-rate*-asserting
   mutation gate.** "No quality signal" outcomes (`total=N unviable=N`,
   `total=0 vacuous pass`) are *categorical* failures that can mask a
   genuine kill-rate regression by short-circuiting the gate before any
   mutants run. The wrapper now treats both classes as gate inputs in
   their own right, not pass-through.
3. **A wrapper that reads its own subprocess's output files must own
   the lifecycle of every file it reads.** The original
   `clear_stale_summary` cleared the *wrapper's* output but trusted the
   *subprocess's* output as fresh. Either contract works; mixing them
   creates the exact stale-read bug surfaced here. Same shape as the
   pattern in `.claude/rules/testing.md` § "Mutation testing is the
   exception" requiring `git checkout -- crates/` after every run —
   *the only valid file on disk is the one the run that just finished
   produced*.
4. **Extracting the error-mapping helper at RED time was the right
   call.** It moved the rustdoc co-located with the variant choice,
   gave the regression test a stable target, and made the GREEN diff a
   four-line variant flip — minimum surface area, minimum revert blast
   radius.

## Quality gates summary

- **DES integrity**: 2 / 2 steps with complete DES traces (PREPARE,
  RED_ACCEPTANCE, RED_UNIT, GREEN, COMMIT each EXECUTED or
  NOT_APPLICABLE-justified). Step 01-01 SKIPPED RED_UNIT and GREEN with
  rationale (RED scaffold step); step 01-02 SKIPPED RED_ACCEPTANCE and
  RED_UNIT with rationale (test already RED from 01-01).
- **Acceptance**: regression test in
  `tests/integration/post_http_invalid_job_id.rs` PASSES under
  `cargo nextest run -p overdrive-cli --features overdrive-cli/integration-tests -E 'binary(integration)'`.
- **Workspace**: `cargo nextest run -p overdrive-cli` green;
  `cargo test --doc -p overdrive-cli` green;
  `cargo clippy -p overdrive-cli --all-targets --features overdrive-cli/integration-tests -- -D warnings`
  clean.
- **Diff scope**: production diff bounded to
  `crates/overdrive-cli/src/commands/job.rs` per the GREEN step
  acceptance criterion. Mutation-gate detour landed as separate commits
  on `xtask/` and `.cargo/mutants.toml` outside the bug-fix scope.
- **Mutation gate**: green after the wrapper fix and the `exclude_re`
  rule landed together in `adcf0bf` + `ec99bc8`. Without both, the
  `Default::default` trade-off would not have been visible to the
  wrapper-driven gate at all (the stale-outcomes bug masked it).

## Follow-ups

None required. The mutation-gate detour landed as part of this PR's
final shape; `.cargo/mutants.toml`'s `skip_calls` entry will be cleaned
up in the next mutants-toml audit (non-blocking).
