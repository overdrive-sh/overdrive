# Root cause analysis — `cargo xtask mutants --diff origin/main` reports PASS while every mutant is unviable

**Branch:** `marcus-sa/phase-1-control-plane-core`
**Date:** 2026-04-25
**Author:** Rex (RCA)
**Status:** investigation complete; fixes proposed; not implemented

---

## Problem statement

CI job `mutants-diff` (`.github/workflows/ci.yml:454-502`) on the
`marcus-sa/phase-1-control-plane-core` branch produced this output:

```
Found 250 mutants to test
INFO Auto-set test timeout to 771s
ok       Unmutated baseline in 190s build + 154s test
250 mutants tested in 7m: 250 unviable
WARN No mutants were viable: perhaps there is a problem with building in a scratch directory.
     Look in mutants.out/log/* for more information.
```

Local reproduction (scoped to one file in the diff) reports:

```
Found 3 mutants to test
ok       Unmutated baseline in 49s build + 25s test
3 mutants tested in 79s: 1 caught, 2 unviable
mutants: mode=diff total=3 caught=1 missed=0 timeout=0 unviable=2 kill_rate=100.0%
mutants: PASS
```

Two distinct failures are in play, and they compound:

- **Failure A** — every mutant in the full-PR run is `unviable` (rustc
  rejects every mutated source file). This is a real signal that
  cargo-mutants' default mutation operators interact badly with this
  workspace's API surface.
- **Failure B** — the gate in `xtask/src/mutants.rs:625-651`
  (`evaluate_diff_gate`) reports **PASS** when `caught == 0 &&
  missed == 0`, regardless of whether `unviable > 0`. The 250-unviable
  CI run reports green; a future PR where every mutant is unviable for
  any reason would also report green silently.

---

## Failure A — every mutant unviable

### Evidence

Local diff-scoped reproduction against
`crates/overdrive-control-plane/src/error.rs` produced 3 mutants. The
two unviable mutants' logs (`target/xtask/mutants.out/log/*.log`)
contain the rustc diagnostics verbatim.

**Mutant 1 — line 52 (`error.rs:52:9`)**

Source under mutation:

```rust
// crates/overdrive-control-plane/src/error.rs:51-53
pub fn internal(context: impl fmt::Display, source: impl fmt::Display) -> Self {
    Self::Internal(format!("{context}: {source}"))
}
```

Mutation operator: `replace ControlPlaneError::internal -> Self with
Default::default()`.

Rustc diagnostic (verbatim from
`target/xtask/mutants.out/log/crates__overdrive-control-plane__src__error.rs_line_52_col_9.log`):

```
error[E0277]: the trait bound `ControlPlaneError: Default` is not satisfied
  --> crates/overdrive-control-plane/src/error.rs:52:9
help: the trait `Default` is not implemented for `ControlPlaneError`
  --> crates/overdrive-control-plane/src/error.rs:18:1
   |
18 | pub enum ControlPlaneError {
```

**Mutant 2 — line 65 (`error.rs:65:5`)**

Source under mutation:

```rust
// crates/overdrive-control-plane/src/error.rs:64-66
#[must_use]
pub fn to_response(err: ControlPlaneError) -> (StatusCode, ErrorBody) {
    use overdrive_core::aggregate::AggregateError;
```

Mutation operator: `replace to_response -> (StatusCode, ErrorBody) with
(Default::default(), Default::default())`.

Rustc diagnostic:

```
error[E0277]: the trait bound `ErrorBody: Default` is not satisfied
   --> crates/overdrive-control-plane/src/error.rs:65:26
help: consider annotating `ErrorBody` with `#[derive(Default)]`
   --> crates/overdrive-control-plane/src/api.rs:174:1
```

This is a hard rustc trait-bound rejection, **not** a workspace-lints
elevation, **not** a trybuild fixture issue, **not** a `--locked` /
manifest issue.

### 5-Whys (Failure A)

```
PROBLEM: 250/250 mutants on the PR's diff are reported `unviable` —
         the baseline builds and tests pass, only the mutated builds
         fail. CI reports PASS via Failure B.

WHY 1: Each mutated source file fails to compile.
  [Evidence: target/xtask/mutants.out/log/*line_52*.log contains
   `error[E0277]: the trait bound ControlPlaneError: Default is not
   satisfied`. The baseline.log compiles successfully. Mutated tmp dir
   path is /private/var/folders/.../cargo-mutants-daegu-v1-HgxdzD.tmp;
   confirmed not a scratch-dir permission problem — rustc reaches type
   resolution before failing.]

  WHY 2: The mutation operator cargo-mutants applies replaces the
         function body with `Default::default()`, but the return type
         (`Self == ControlPlaneError`, `(StatusCode, ErrorBody)`,
         `Job`, etc.) does not implement `Default`.
    [Evidence: line_52 mutation diff shows `Self::Internal(format!(...))`
     replaced by removal of the body (the operator emits an empty body
     whose return type is implicitly `Default::default()` at the
     trait-resolution level — log line shows
     `replace ControlPlaneError::internal -> Self with Default::default()`).
     line_65 shows `(Default::default(), Default::default())` for the
     `(StatusCode, ErrorBody)` return — `ErrorBody` is defined at
     `crates/overdrive-control-plane/src/api.rs:174` without
     `#[derive(Default)]`.]

    WHY 3: cargo-mutants' default operator catalogue
           (https://mutants.rs/operators.html) includes "replace
           function body with Default::default()" as a near-universal
           mutation; on a workspace where most domain types
           deliberately do NOT implement `Default` (newtypes per
           development.md "STRICT by default", typed errors per
           thiserror conventions, sum types modelling business state),
           the operator is rejected by rustc at every site.
      [Evidence: development.md §Newtypes — STRICT by default lists
       JobId, AllocationId, SpiffeId, etc. as newtypes whose
       constructors *must* validate and return Result; this rules out
       a free Default impl. .claude/rules/development.md §Errors:
       "thiserror enums in libraries" — ControlPlaneError is a
       thiserror enum with no Default. Confirmed by inspection of
       error.rs:17-39 (no `#[derive(Default)]`) and api.rs:174 (no
       Default on ErrorBody).]

      WHY 4: cargo-mutants emits the Default::default() operator
             unconditionally because, on average across Rust
             ecosystems, types do tend to implement Default — the
             operator's design assumes a high "viable" rate. The
             xtask wrapper does not pass `--cap-lints=warn` or any
             other flag that affects rustc's trait resolution; even
             if it did, E0277 is a hard error, not a lint.
        [Evidence: build_cargo_mutants_args at xtask/src/mutants.rs:249-346
         emits no per-operator filter. additional_cargo_args in
         .cargo/mutants.toml:180 sets only `--locked`. No
         `skip_calls = ["Default::default"]` or equivalent.
         cargo-mutants 25.x docs at https://mutants.rs/skip_calls.html
         confirm `skip_calls` is the supported mechanism for muting
         specific replacement values.]

        WHY 5: The project did not declare which mutation operators
               apply to its API surface — the default catalogue is in
               effect. On a Phase 1 codebase whose domain layer is
               deliberately allergic to Default (newtypes, typed
               errors, sum types), the default catalogue is the wrong
               default — most mutations are not "missed" or "caught,"
               they are "unviable," and unviable is not a quality
               signal.
          [Evidence: .cargo/mutants.toml `skip_calls = []` at line 190;
           comment at line 188-190 says "we deliberately leave this
           empty so the full operator set runs." This is a deliberate
           project choice that has now compounded into a 100%-unviable
           PR-scale signal.]

ROOT CAUSE A: The default cargo-mutants operator catalogue includes
              "replace function body with Default::default()" applied
              indiscriminately to every function. The Phase 1 surface
              has a high density of return types (newtypes, thiserror
              enums, sum types) that deliberately do not implement
              Default per project convention. Result: rustc rejects
              the mutation at trait-resolution time before any test
              runs, and cargo-mutants records the mutant as Unviable.
              At the scale of the full PR diff (250 mutants on a
              43,041-insertion branch), every Default-replacement
              mutation hit a non-Default return type, and the cargo-
              mutants summary reports `caught=0 missed=0 unviable=250`.

  Note: this does NOT mean every mutation operator is broken. Other
  operators (boolean negation, comparison swap, arm deletion in
  match, etc.) remain useful. The fix is to filter the offending
  operator, not to disable mutation testing.

Eliminated alternatives:

  Hypothesis 1 — workspace-lints elevate mutation-induced warnings to
  errors. Ruled out by the actual rustc diagnostic: `error[E0277]: the
  trait bound X: Default is not satisfied` is a trait-resolution error,
  not a lint. The workspace lints at Cargo.toml:120-145 are
  pedantic/nursery (warn) plus targeted dbg_macro/print_*/todo (warn or
  deny). None of these elevate E0277. Verified by reading the unviable
  mutant logs end-to-end.

  Hypothesis 2 — a compile_fail / trybuild fixture is mutated and
  perversely compiles. Ruled out by `git diff --name-only origin/main
  -- 'crates/overdrive-core/tests/compile_fail/'` returning empty (the
  three fixtures at crates/overdrive-core/tests/compile_fail/ are NOT
  in the PR diff), and by .cargo/mutants.toml:76 excluding `**/tests/**`
  from mutation regardless.

  Hypothesis 3 — `--locked` interacts with mutated Cargo.toml. Ruled
  out by inspection of the unviable logs: the mutation diff shows only
  .rs file changes, not Cargo.toml changes. cargo-mutants' --in-diff
  filter lands only on mutations whose source file appears in the
  diff; it does not mutate manifests.

  Hypothesis 4 — `unsafe_op_in_unsafe_fn = deny` interacts with
  synthesized unsafe calls. Ruled out by the diagnostic — E0277 (trait
  bound) is unrelated to unsafe-op lints. None of the four crates
  forbid unsafe (they `#![forbid(unsafe_code)]`); cargo-mutants does
  not synthesize unsafe blocks.
```

### Backwards chain validation

If ROOT CAUSE A holds:

- Every function whose return type lacks `Default` will produce an
  unviable mutant for the `Default::default()` operator. ✅ Confirmed
  by both observed mutants (line_52 returns `Self == ControlPlaneError`,
  line_65 returns `(StatusCode, ErrorBody)`).
- The `caught` line_117 mutant should target a function whose body
  does NOT involve Default — confirmed: line_117 is
  `IntoResponse::into_response`, where the only mutation that fits
  cargo-mutants' catalogue is one the existing tests catch.
- Functions returning `()` should still be viable (they accept
  `Default::default()` since `() == <() as Default>::default()`). ✅
  Consistent with `.cargo/mutants.toml:139` already excluding such an
  equivalent mutant on `NoopHeartbeat::hydrate`.
- The CI-scale "250/250 unviable" must mean either every mutation
  operator selected for this diff is the Default replacement, OR the
  population of selectable functions on the diff is overwhelmingly
  Default-incompatible. ✅ Phase 1 control-plane code is mostly
  thiserror enums + newtypes + axum handlers returning custom types;
  this matches the population observed in the local 3-mutant scoped
  reproduction (2 of 3 hit Default mutations, 1 passed).

---

## Failure B — gate scoring bug

### Evidence

`xtask/src/mutants.rs:625-651`:

```rust
fn evaluate_diff_gate(report: &RawReport) -> Gate {
    let kill_rate_pct = kill_rate_percent(report.caught, report.missed);
    let status = if report.caught == 0 && report.missed == 0 {
        // No gate-relevant mutations generated from the diff — e.g. the
        // PR only touched excluded paths or comments. Vacuously pass.
        GateStatus::Pass
    } else if kill_rate_pct + f64::EPSILON < DIFF_KILL_RATE_FLOOR {
        GateStatus::Fail { reason: ... }
    } else {
        GateStatus::Pass
    };
    ...
}
```

`kill_rate_percent` at `xtask/src/mutants.rs:590-600`:

```rust
fn kill_rate_percent(caught: u64, missed: u64) -> f64 {
    let denom = caught.saturating_add(missed);
    if denom == 0 { 100.0 } else { ... }
}
```

Existing tests that explicitly enforce the (incorrect) behaviour:

- `xtask/src/mutants.rs:881-884` —
  `kill_rate_is_vacuously_100_when_no_mutations_evaluated` asserts
  `kill_rate_percent(0, 0) == 100.0`. This is fine in isolation; the
  bug is in how `evaluate_diff_gate` consumes it.
- `xtask/src/mutants.rs:908-914` —
  `diff_gate_passes_when_diff_generated_no_mutations` calls
  `report(0, 0, 5, 0)` (5 unviable, 0 caught, 0 missed) and asserts
  `GateStatus::Pass`. Comment claims "PR touched only excluded paths."

The CI output reproduces this exactly: `total=250 caught=0 missed=0
unviable=250` → kill_rate_pct = 100.0 → `caught == 0 && missed == 0`
→ `GateStatus::Pass`. The "only excluded paths" interpretation is
indistinguishable at the predicate level from "every mutant unviable."

### 5-Whys (Failure B)

```
PROBLEM: The gate predicate cannot distinguish "no mutants in scope"
         (legitimate vacuous pass) from "every mutant unviable"
         (mutation testing produced no quality signal at all).

WHY 1: The vacuous-pass branch fires whenever caught + missed == 0,
       independent of whether unviable > 0.
  [Evidence: xtask/src/mutants.rs:627-630 — predicate is
   `report.caught == 0 && report.missed == 0`. `unviable` is not
   consulted. Total is not consulted.]

  WHY 2: The denominator of kill rate excludes unviable by design,
         per the doc comment at xtask/src/mutants.rs:24-33: "Kill
         rate is `caught / (caught + missed)` — the denominator
         excludes Unviable and Timeout." This is correct as a
         *kill rate* definition. The bug is treating "kill rate
         denominator empty" as equivalent to "nothing to test."
    [Evidence: kill_rate_percent at line 590-600 returns 100.0 when
     denom == 0 — a deliberate sentinel. The doc comment justifies
     it for a real reason: "no mutants in scope is vacuously 100%."
     But evaluate_diff_gate consumes the sentinel without
     disambiguating it from the all-unviable case.]

    WHY 3: The vacuous-pass branch was originally written to handle
           PRs that touch only excluded paths (xtask/, generated
           code, tests). In that case `total_mutants == 0`. But
           cargo-mutants reports `total_mutants = caught + missed +
           unviable + timeout`, and an all-unviable run has
           `total_mutants = 250` (the CI output's "Found 250 mutants
           to test"). The gate's predicate omits `total_mutants` and
           omits `unviable`.
      [Evidence: comment at xtask/src/mutants.rs:628-629 says "No
       gate-relevant mutations generated from the diff — e.g. the PR
       only touched excluded paths or comments. Vacuously pass." The
       intent is "total_mutants == 0," but the implementation reads
       "kill-rate denominator == 0," which is broader. Test at
       line 908-914 misleadingly names the case
       `diff_gate_passes_when_diff_generated_no_mutations` but
       actually feeds it `(caught=0, missed=0, unviable=5, timeout=0)`
       — total=6, not zero. The test thus already encodes the
       all-unviable case as a PASS, locking in the wrong invariant.]

      WHY 4: When the gate was written (commit history shows
             xtask/src/mutants.rs is part of the diff under review;
             the doc comment at line 25-40 justifies the kill-rate
             arithmetic against an old gate that "computed
             caught / len(outcomes) over a field cargo-mutants does
             not emit"), the author correctly wanted to allow
             excluded-paths PRs to pass and correctly wanted to
             exclude unviable from the kill-rate denominator. The
             missing third invariant — "unviable does not displace
             real test signal" — was not encoded.
        [Evidence: the function's only check on `unviable` is
         informational (printed in the human-readable line and
         summary JSON). It is never consulted in any gate predicate.
         Same applies to `timeout`: it is reported but never gated.]

        WHY 5: cargo-mutants' own warning channel
               ("WARN No mutants were viable: perhaps there is a
               problem with building in a scratch directory.") is
               surfaced to the operator's console, but the xtask
               wrapper does not parse it back from cargo-mutants'
               output and does not include it in the gate. The
               outcomes.json schema (RawReport at
               xtask/src/mutants.rs:558-575) carries `total_mutants`
               and `unviable` as separate fields; the gate logic
               could disambiguate but does not.
          [Evidence: parse_outcomes at xtask/src/mutants.rs:577-585
           deserialises `unviable` and `total_mutants` correctly
           (verified by raw_report_deserialises_from_cargo_mutants_output
           at line 1003-1030, which deserialises a real outcomes.json
           with `unviable: 73`). The fields exist in the report at
           gate-evaluation time. The gate just doesn't read them.]

ROOT CAUSE B: The diff-mode gate predicate at
              xtask/src/mutants.rs:627 conflates "no mutants in scope"
              with "all mutants unviable" because it inspects only
              `caught == 0 && missed == 0`. The `unviable` and
              `total_mutants` counters available in the parsed report
              are not consulted. The locked-in test at line 908-914
              was named "diff_generated_no_mutations" but actually
              encodes the all-unviable case as a PASS, so the bug
              ships behind a green test.
```

### Backwards chain validation

If ROOT CAUSE B holds:

- A run with `caught=0, missed=0, unviable=250, total=250` reports
  PASS. ✅ Reproduced by the CI output cited at the top of this doc
  ("250 mutants tested in 7m: 250 unviable" → "mutants: PASS").
- A run with `caught=0, missed=0, unviable=0, total=0` (legitimate
  vacuous pass) ALSO reports PASS. ✅ Same predicate, different
  semantics. Both reach the vacuous-PASS branch through the same
  gate, which is the bug.
- The unit test at `xtask/src/mutants.rs:908-914` should fail under a
  corrected gate. ✅ It will: it feeds `(0, 0, 5, 0)` and asserts
  PASS, which is the locked-in wrong invariant.

### Cross-validation

Root Cause A and Root Cause B are independent and compose multiplicatively:

- Without B: A would have failed CI loudly (gate would have refused
  to score 0/0 as 100%) and the operator would have been forced to
  diagnose A. The bug would have been visible in the first PR where
  it manifested.
- Without A: B would have remained latent until *something else*
  produced an all-unviable run (a future cargo-mutants version
  change, a workspace-wide manifest issue, an environment break) —
  and would silently report PASS at that point.

A produced the failure; B hid it. Both must be fixed; B is the more
load-bearing fix because it prevents future masking regardless of
which underlying cause produces all-unviable runs.

---

## Fix proposal

### Fix A — narrow the cargo-mutants operator catalogue

The `replace function body with Default::default()` operator is the
specific source of unviability on this codebase. cargo-mutants exposes
two mechanisms for filtering it (https://mutants.rs/skip_calls.html
and https://mutants.rs/operators.html#skipping-operators):

1. `skip_calls = ["Default::default"]` in `.cargo/mutants.toml`
   suppresses any mutation that *replaces* a function body with a
   call to `Default::default()`.
2. `additional_cargo_args = ["--locked"]` already exists at
   `.cargo/mutants.toml:180`; appending an `--exclude-re` value at
   the cargo-mutants level is a viable alternative if `skip_calls`
   is too coarse.

**Recommended change** (single edit, `.cargo/mutants.toml:190`):

```toml
# Replace
skip_calls = []

# With
skip_calls = ["Default::default"]
```

Rationale: the workspace's domain types deliberately do not implement
`Default` (newtypes per `.claude/rules/development.md` §Newtypes —
STRICT by default; thiserror enums per §Errors). The
`Default::default()` operator therefore has near-zero viable yield on
this codebase and produces only `unviable` outcomes. Excluding it
preserves the full set of remaining operators (boolean negation,
comparison swap, arm deletion, etc.) which are the high-signal
operators per https://mutants.rs/operators.html.

The exclusion comment in `.cargo/mutants.toml:188-190` already
acknowledges `skip_calls` as the project's chosen granularity for
operator-level exclusions; adding `Default::default` follows the
existing pattern.

**Sanity check before landing:** run

```
cargo xtask mutants --diff origin/main \
  --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/error.rs
```

Expected: 1 mutant (the previously-caught one at line 117), kill rate
100% on a 1-mutant denominator. The two Default-replacement mutants
should be filtered out at cargo-mutants' generation step and not
appear in `Found N mutants` at all.

If the per-file local run still shows unviable mutants from
*non-Default* operators, those are real signals — investigate
individually rather than widening `skip_calls`.

### Fix B — gate must distinguish all-unviable from no-mutants

Two-line change to `xtask/src/mutants.rs:627-642`:

```rust
fn evaluate_diff_gate(report: &RawReport) -> Gate {
    let kill_rate_pct = kill_rate_percent(report.caught, report.missed);
    let status = if report.total_mutants == 0 {
        // No mutants generated — PR touched only excluded paths or
        // comments. Vacuously pass. (Was: caught == 0 && missed == 0,
        // which conflated this with the all-unviable case.)
        GateStatus::Pass
    } else if report.caught == 0 && report.missed == 0 {
        // total_mutants > 0 but kill-rate denominator is zero —
        // every mutant was unviable or timed out. Mutation testing
        // produced no quality signal. Refuse the gate.
        GateStatus::Fail {
            reason: format!(
                "mutants produced no quality signal: total={} \
                 unviable={} timeout={} caught=0 missed=0 — \
                 see target/xtask/mutants.out/log/* for the rustc \
                 diagnostics on each unviable mutant",
                report.total_mutants, report.unviable, report.timeout,
            ),
        }
    } else if kill_rate_pct + f64::EPSILON < DIFF_KILL_RATE_FLOOR {
        GateStatus::Fail {
            reason: format!(
                "mutants kill rate {kill_rate_pct:.1}% < {DIFF_KILL_RATE_FLOOR:.1}% threshold \
                 (caught={caught} missed={missed})",
                caught = report.caught,
                missed = report.missed,
            ),
        }
    } else {
        GateStatus::Pass
    };
    ...
}
```

Test changes:

1. **Replace** the misleading
   `diff_gate_passes_when_diff_generated_no_mutations` at line 908-914.
   It currently feeds `report(0, 0, 5, 0)` (5 unviable, 0 caught, 0
   missed, total = 6 per the test helper at line 862-872 which sets
   `total_mutants = caught + missed + unviable + timeout + 1`). The
   helper is already wrong for the "no mutants" case — total is never
   zero.

   Split into two tests:

   ```rust
   #[test]
   fn diff_gate_passes_when_no_mutants_in_scope() {
       // Truly no mutants generated (PR touched only excluded paths).
       // total=0 is the only correct shape for this case.
       let r = RawReport {
           total_mutants: 0,
           caught: 0, missed: 0, unviable: 0, timeout: 0,
           success: 1,
           cargo_mutants_version: "27.0.0".into(),
       };
       assert!(matches!(evaluate_diff_gate(&r).status, GateStatus::Pass));
   }

   #[test]
   fn diff_gate_fails_when_every_mutant_unviable() {
       // 250 mutants generated, all unviable. No quality signal.
       // CI saw exactly this on phase-1-control-plane-core; gate
       // must refuse.
       let r = RawReport {
           total_mutants: 250,
           caught: 0, missed: 0, unviable: 250, timeout: 0,
           success: 1,
           cargo_mutants_version: "27.0.0".into(),
       };
       let reason = match evaluate_diff_gate(&r).status {
           GateStatus::Fail { reason } => reason,
           other => panic!("expected Fail, got {other:?}"),
       };
       assert!(reason.contains("no quality signal"), "got: {reason}");
       assert!(reason.contains("unviable=250"), "got: {reason}");
   }
   ```

2. The existing `report()` helper at `xtask/src/mutants.rs:862-872`
   adds 1 to total to dodge a divide-by-zero edge case in some unit
   tests. After Fix B, `total_mutants == 0` is a meaningful gate
   input, so the helper's `+1` becomes load-bearing for the wrong
   tests. Keep the helper for existing call sites; the two new tests
   above construct `RawReport` directly without the helper.

3. Optional: add a test for the workspace gate
   (`evaluate_workspace_gate`) with the same shape — it has the same
   structural risk via `kill_rate_percent` returning 100.0 for an
   empty denominator.

### Validation

After both fixes land, the canonical reproduction shape should now
behave as follows:

```
# Before A+B: PASS (250 unviable masked by vacuous-pass gate)
# After A+B:
cargo xtask mutants --diff origin/main
# Expected:
#   - cargo-mutants reports a smaller `Found N mutants` (Default
#     replacements suppressed at generation)
#   - remaining mutants run; kill rate is the real number against the
#     diff-scoped suite
#   - if any latent code path still produces all-unviable, the gate
#     fails with "mutants produced no quality signal: ..." instead
#     of silently passing
```

Per-file repro to land alongside the fix:

```
cargo xtask mutants --diff origin/main \
  --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/error.rs
# Expected: 1 mutant (line 117 IntoResponse), 100% killed
```

The new test `diff_gate_fails_when_every_mutant_unviable` at
`xtask/src/mutants.rs` (added per Fix B) is the regression guard. It
fails on the current code; it must pass after Fix B and never be
deleted without an ADR.

---

## Prevention

- **Document the operator filter rationale** in `.cargo/mutants.toml`
  next to the new `skip_calls` entry so a future contributor doesn't
  re-enable the operator without re-running the diagnostic.
- **Add a smoke check in CI** that `unviable / total_mutants < 0.5`
  on the diff-scoped run. This is a separate signal from the gate —
  a high unviable ratio means the operator catalogue is mismatched
  to the codebase, even when caught/missed are healthy. Could land
  as a `GateStatus::Warn` annotation rather than a hard fail.
- **Cross-reference** in `.claude/rules/testing.md` §"Mutation testing
  (cargo-mutants)" §"Reading the output" that the summary's
  `unviable` field is the diagnostic, and that all-unviable is now
  Fail (not Pass).

---

## Files modified by this proposal (NOT yet implemented)

- `.cargo/mutants.toml:190` — add `"Default::default"` to `skip_calls`
- `xtask/src/mutants.rs:627-651` — replace gate predicate to consult
  `total_mutants` and `unviable`
- `xtask/src/mutants.rs:908-914` — replace
  `diff_gate_passes_when_diff_generated_no_mutations` with the two
  tests sketched above

Optional follow-ups:

- `xtask/src/mutants.rs` `evaluate_workspace_gate` — same structural
  fix
- `.github/workflows/ci.yml:454-502` — surface the unviable ratio in
  the step summary (informational)
- `.claude/rules/testing.md` §"Mutation testing (cargo-mutants)" —
  document the all-unviable Fail case and the `skip_calls` rationale
