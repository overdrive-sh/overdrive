# RCA — `cargo xtask mutants` crashes on zero-mutant filter intersection

**Defect**: When `cargo xtask mutants --diff origin/main --package xtask
--file xtask/src/dst_lint.rs` is invoked and cargo-mutants' filter
intersection (`--in-diff` ∩ `--file` ∩ `--package`) produces zero
candidate mutants, cargo-mutants logs `INFO No mutants to filter`,
exits 0, and does **not** create `target/xtask/mutants.out/` or
`outcomes.json`. The xtask wrapper unconditionally attempts to parse
`outcomes.json` afterwards and aborts with
`bail!("no outcomes.json at … — cargo-mutants did not produce a report")`.

The kill-rate gate is **vacuously satisfied** in this state — there
is nothing to evaluate — so the wrapper should exit 0 with a
"no mutants in scope" annotation, not crash.

**Repro context**:
- Branch: `marcus-sa/phase-1-control-plane-core` against `origin/main`.
- Diff is 2.7 MB; intersected with `--file xtask/src/dst_lint.rs` and
  `--package xtask`, the candidate mutant set is empty (no diff lines
  in that file map onto a mutable mutation operator within the scoped
  package).
- Two earlier mutation runs in the same session
  (`overdrive-control-plane` 90.9%, `overdrive-sim` 100.0%) succeeded
  cleanly — those runs had non-empty filter intersections.
- cargo-mutants 27.x (workspace-pinned).

---

## Root cause chain (5 Whys, multi-causal)

Three independent root causes each have to hold for the crash to
manifest.

### Branch A — wrapper-side parsing assumption

```
WHY 1A  parse_outcomes bails because outcomes.json is absent.
        [xtask/src/mutants.rs:578 — `if !path.is_file() { bail!(...) }`]

WHY 2A  parse_outcomes is called unconditionally after invoke_cargo_mutants.
        [xtask/src/mutants.rs:138-148 — no zero-mutant-skip branch
         between subprocess return and the parse step.]

WHY 3A  invoke_cargo_mutants discards the exit status (`let _ = ...?`)
        and has no other signal to distinguish a successful no-op from
        a crash. The only fallback is the wrap_err string at L143-147.
        [xtask/src/mutants.rs:138 + comment L134-137]

WHY 4A  The design models two outcomes only — "ran the loop, produced
        outcomes.json" or "crashed before writing". The third state
        — "ran successfully, short-circuited before writing" — is
        unmodelled.
        [xtask/src/mutants.rs:625-666 — every gate branch is downstream
         of a successful parse_outcomes; there is no `Option<RawReport>`
         path.]

WHY 5A  ROOT — The wrapper conflates two distinct file-absence
        semantics: (a) cargo-mutants crashed; (b) cargo-mutants
        successfully short-circuited. Both reach the same
        parse_outcomes failure; only (a) is a true error.
        [Comment at xtask/src/mutants.rs:143-147 explicitly says
         "cargo-mutants may have crashed before writing its report" —
         revealing the design only modelled the crash case.]
```

**ROOT CAUSE A**: The wrapper has no zero-mutant-no-report branch.
`parse_outcomes` treats a missing file as fatal; the code path entered
when cargo-mutants exits 0 with an empty filter intersection has no
representation in the wrapper's state machine.

### Branch B — no positive success signal

```
WHY 1B  The wrapper cannot detect cargo-mutants' clean exit; it
        consults only file presence as a proxy for "ran".

WHY 2B  Exit status was deliberately discarded so that "missed
        mutants ⇒ non-zero exit" propagates as gate decisions
        rather than wrapper crashes.
        [xtask/src/mutants.rs:134-137 comment]

WHY 3B  cargo-mutants overloads its exit status (0 = all caught,
        non-zero = something missed). The wrapper substituted
        "did outcomes.json appear?" as the stronger signal — sound
        IF "ran to completion ⇒ wrote a file", which the empty-filter
        case violates.

WHY 4B  cargo-mutants emits `INFO No mutants to filter` and exits 0
        *without* creating mutants.out/ when the filter intersection
        is empty. This third exit class is not distinguishable from
        "crashed before writing".

WHY 5B  ROOT — There is no machine-readable channel from cargo-mutants
        that says "I successfully had nothing to do". stdout text
        ("INFO No mutants to filter") is the only marker; exit code
        is the same 0 a normal all-caught run produces.
```

**ROOT CAUSE B**: cargo-mutants' upstream "filter produced zero
mutants" code path is a silent short-circuit — exit 0, log line on
stdout/stderr, no report file. The wrapper cannot discriminate this
from a crash because cargo-mutants does not provide a structured
signal.

### Branch C — test coverage gap

```
WHY 1C  A test should have caught this; none did.
        [xtask/src/mutants.rs:912-1099 — all gate tests construct
         RawReport in-memory; none routes through parse_outcomes
         against an absent file.]

WHY 2C  The unit suite asserts on synthetic RawReport values directly,
        skipping the parse_outcomes layer entirely.

WHY 3C  Subprocess integration is deferred to CI ("subprocess-level
        integration … is covered by the CI workflows themselves",
        comment L890-893). CI never exercises empty filter
        intersection — recent commits (4a6e82a, d2df6a3, 35bc7c0)
        target full-diff or full-workspace shapes only.

WHY 4C  The fixture at L1102-1128 was "verified against an actual
        outcomes.json from this repo" — but only the populated-report
        shape was captured. The absent-report shape was never sampled
        because no developer previously hit it interactively.

WHY 5C  ROOT — The wrapper-↔-cargo-mutants contract is encoded only
        in inline comments (L134-137, L143-147), never as a test
        asserting "an empty filter intersection is recoverable".
```

**ROOT CAUSE C**: The wrapper-↔-cargo-mutants contract lives in
inline comments, never as an executable assertion. The first time
the assumption was wrong (this bug) it crashed in production.

### Cross-validation

| Premise | Observed symptom |
|---|---|
| A holds (no zero-mutant-no-report branch) | parse_outcomes bail is the surfaced error ✓ |
| B holds (cargo-mutants short-circuits without writing) | file-absence trigger fires ✓ |
| C holds (no test simulates absent-outcomes) | bug ships unnoticed until human triggers it ✓ |

All three are required: without B the file would always be written;
without A the absence would be handled gracefully; without C either
A or B would have been caught pre-merge. Two prior runs in the same
session (90.9%, 100.0%) succeeded because their filter intersections
were non-empty — they never reached the short-circuit.

---

## Contributing factors

1. **cargo-mutants 27.x default**: the empty-filter short-circuit
   does not write a stub `outcomes.json` (e.g.
   `{"total_mutants":0,"caught":0,"missed":0,...}`). A future
   upstream PR could change this; the wrapper would be more robust
   if it did not depend on the choice.

2. **Wrapper's "summary file as authoritative" contract**: the
   contract documented in `.claude/rules/testing.md` § "Mutation
   testing (cargo-mutants)" → "Reading the output" promises
   `target/xtask/mutants-summary.json` is the structured gate
   record. Today, on the empty-filter path, the file is never
   written (the wrapper crashes before reaching `write_summary` at
   L157). CI consumers polling the summary file see the prior
   run's verdict (or nothing) — not a representation of "this run
   had nothing to do".

3. **`clear_stale_summary` runs *before* the subprocess** (L132):
   correct for the crash-mid-run case, but compounds the issue
   here — the empty-filter run leaves no summary on disk at all,
   so a CI step parsing `target/xtask/mutants-summary.json` fails
   independently of the wrapper's exit code.

4. **Per-step DELIVER discipline encourages narrow `--file`
   scopes**: `.claude/rules/testing.md` § "Per-step vs per-PR
   scoping" actively recommends invoking the wrapper with a single
   `--file` per delivery step. This makes the empty-intersection
   case the *common* shape during inner-loop development, not an
   edge case.

---

## Proposed fix

Add a zero-mutant-no-report branch to `run()` that:
- Detects "subprocess exited successfully *and* no `outcomes.json`
  was written" → treats the run as vacuously passing.
- Writes a `mutants-summary.json` with `total_mutants=0`,
  `caught=0`, `missed=0`, `status="pass"`, `reason="no mutants in
  scope"` so downstream consumers (CI, future tooling) see a
  representation of the run rather than a missing file.
- Re-introduces partial exit-status checking: a non-zero exit
  combined with absent `outcomes.json` remains a hard error
  (subprocess crash). Only `exit==0 && file absent` is the
  vacuous-pass path.

### File 1 — `xtask/src/mutants.rs`

#### Change 1.1 — capture the exit status

Replace the discarded-exit-status pattern at L138 with a binding
the caller can inspect.

**Before** (L138):
```rust
    // 1. Run cargo-mutants. Exit status is intentionally ignored — a
    //    non-zero exit from cargo-mutants happens on any missed mutant,
    //    which we handle via our own gate below. We only care that the
    //    subprocess produced `outcomes.json`.
    let _ = invoke_cargo_mutants(mode, scope, &output_parent)?;

    // 2. Parse outcomes.json.
    let outcomes_path = out_dir.join("outcomes.json");
    let report = parse_outcomes(&outcomes_path).wrap_err_with(|| {
        format!(
            "parse cargo-mutants outcomes at {} — cargo-mutants may have crashed \
             before writing its report",
            outcomes_path.display()
        )
    })?;
```

**After**:
```rust
    // 1. Run cargo-mutants. Exit status is partially trusted — a
    //    non-zero exit happens both for missed mutants (our gate
    //    handles it) and for genuine subprocess crashes. We
    //    distinguish them via the report file: a clean run always
    //    writes `outcomes.json`, *unless* cargo-mutants
    //    short-circuited at filter time ("INFO No mutants to filter")
    //    — in which case it exits 0 and writes nothing. That third
    //    state is treated as a vacuously-passing zero-mutant run.
    let status = invoke_cargo_mutants(mode, scope, &output_parent)?;

    // 2. Parse outcomes.json — or, if absent AND the subprocess
    //    exited cleanly, treat as a zero-mutant short-circuit.
    let outcomes_path = out_dir.join("outcomes.json");
    let report = match read_outcomes_or_short_circuit(&outcomes_path, status)? {
        Some(report) => report,
        None => {
            // cargo-mutants exited 0 but wrote nothing — the
            // "INFO No mutants to filter" path. Vacuously pass:
            // emit a synthetic zero-mutant report, write the
            // summary, print one line, exit 0.
            return finalise_zero_mutant_run(&summary_path, mode);
        }
    };
```

#### Change 1.2 — add the discriminator helper

Add immediately after `parse_outcomes` (around L585):

```rust
/// Discriminate between three post-subprocess states:
///   1. outcomes.json present  → parse and return Some(report).
///   2. outcomes.json absent + exit 0 → short-circuit (filter
///      intersection empty); return None and let the caller emit
///      a synthetic zero-mutant report.
///   3. outcomes.json absent + non-zero exit → genuine crash; bail
///      with the original "may have crashed" error.
///
/// The exit-status check is the only thing distinguishing (2) from
/// (3); cargo-mutants does not expose a structured "I had nothing
/// to do" signal.
fn read_outcomes_or_short_circuit(
    path: &Path,
    status: std::process::ExitStatus,
) -> Result<Option<RawReport>> {
    if path.is_file() {
        return Ok(Some(parse_outcomes(path)?));
    }
    if status.success() {
        // Filter intersection produced zero mutants. cargo-mutants
        // logged "INFO No mutants to filter" and returned 0 without
        // creating mutants.out/. Vacuously pass.
        eprintln!(
            "xtask mutants: cargo-mutants produced no report (filter \
             intersection is empty) — treating as zero-mutant vacuous pass"
        );
        return Ok(None);
    }
    bail!(
        "no outcomes.json at {} — cargo-mutants exited {} without producing \
         a report (subprocess likely crashed)",
        path.display(),
        status.code().map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
    )
}
```

#### Change 1.3 — emit a synthetic summary for the zero-mutant case

Add immediately after `read_outcomes_or_short_circuit`:

```rust
/// Emit a `mutants-summary.json` representing a vacuously-passing
/// zero-mutant run, print a one-line report, and return Ok(()) so
/// the wrapper exits 0. Mirrors `write_summary` + `print_report`
/// for the case where there is no `RawReport` to populate them
/// from.
fn finalise_zero_mutant_run(summary_path: &Path, mode: &Mode) -> Result<()> {
    let synthetic = RawReport {
        total_mutants: 0,
        caught: 0,
        missed: 0,
        timeout: 0,
        unviable: 0,
        success: 0,
        cargo_mutants_version: String::new(),
    };
    let gate = match mode {
        Mode::Diff { .. } => evaluate_diff_gate(&synthetic),
        // Workspace mode never short-circuits to zero mutants in
        // practice (no --in-diff filter), but model it consistently:
        // the diff-gate logic already returns Pass on total_mutants=0.
        Mode::Workspace { .. } => evaluate_diff_gate(&synthetic),
    };
    write_summary(summary_path, &synthetic, &gate, mode)
        .wrap_err_with(|| format!("write {}", summary_path.display()))?;
    println!(
        "mutants: mode={} total=0 — no mutants in scope (filter intersection empty); vacuous pass",
        gate.mode_label
    );
    println!("mutants: PASS");
    Ok(())
}
```

#### Change 1.4 — add tests for the new path

Add inside the existing `mod tests` (after the existing
`raw_report_deserialises_from_cargo_mutants_output` test):

```rust
    #[test]
    fn read_outcomes_returns_none_when_file_absent_and_exit_zero() {
        // Simulate cargo-mutants short-circuiting on "No mutants to filter":
        // outcomes.json absent, exit 0 ⇒ Ok(None).
        let tmp = tempfile::tempdir().expect("tempdir");
        let absent = tmp.path().join("outcomes.json");
        let zero = std::process::Command::new("true")
            .status()
            .expect("spawn true");
        let result = read_outcomes_or_short_circuit(&absent, zero).expect("must not bail");
        assert!(result.is_none(), "absent file + clean exit ⇒ None");
    }

    #[test]
    fn read_outcomes_bails_when_file_absent_and_exit_nonzero() {
        // Crash case: outcomes.json absent AND subprocess returned
        // non-zero. Must bail.
        let tmp = tempfile::tempdir().expect("tempdir");
        let absent = tmp.path().join("outcomes.json");
        let nonzero = std::process::Command::new("false")
            .status()
            .expect("spawn false");
        let err = read_outcomes_or_short_circuit(&absent, nonzero)
            .expect_err("must bail on absent file + non-zero exit");
        let msg = format!("{err:#}");
        assert!(msg.contains("no outcomes.json"), "got: {msg}");
        assert!(msg.contains("subprocess likely crashed"), "got: {msg}");
    }

    #[test]
    fn read_outcomes_returns_some_when_file_present() {
        // Happy path: file present ⇒ parse and return Some(report).
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("outcomes.json");
        std::fs::write(
            &path,
            r#"{"outcomes":[],"total_mutants":1,"caught":1,"missed":0,
                "timeout":0,"unviable":0,"success":1,
                "cargo_mutants_version":"27.0.0"}"#,
        ).unwrap();
        let zero = std::process::Command::new("true").status().expect("spawn true");
        let report = read_outcomes_or_short_circuit(&path, zero)
            .expect("must parse")
            .expect("Some when file present");
        assert_eq!(report.caught, 1);
    }

    #[test]
    fn finalise_zero_mutant_run_writes_pass_summary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let summary = tmp.path().join("mutants-summary.json");
        finalise_zero_mutant_run(&summary, &Mode::Diff { base: "origin/main".into() })
            .expect("must succeed");
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&summary).unwrap()).unwrap();
        assert_eq!(v["status"].as_str(), Some("pass"));
        assert_eq!(v["total_mutants"].as_u64(), Some(0));
        assert_eq!(v["caught"].as_u64(), Some(0));
        assert_eq!(v["missed"].as_u64(), Some(0));
        // 100.0 because kill_rate_percent(0, 0) == 100.0 by the
        // existing vacuous-pass convention.
        assert!((v["kill_rate_pct"].as_f64().unwrap() - 100.0).abs() < 1e-6);
        assert_eq!(v["base_ref"].as_str(), Some("origin/main"));
    }
```

### File 2 — `.claude/rules/testing.md` (documentation update)

Add a single paragraph under § "Mutation testing (cargo-mutants)" →
"Per-step vs per-PR scoping" that documents the zero-mutant outcome.
This closes Root Cause C by promoting the contract from comments
into prose:

> **Empty filter intersection is a vacuous pass.** When `--file` (or
> `--file` × `--diff`) names paths whose diff lines do not overlap a
> mutable mutation operator, cargo-mutants logs `INFO No mutants to
> filter` and exits 0 without producing `outcomes.json`. The wrapper
> treats this as a vacuous pass — kill rate is undefined, the gate
> is satisfied, and `target/xtask/mutants-summary.json` records
> `total_mutants=0` with `status="pass"`. If you expected mutants
> and got zero, double-check that the file actually changed against
> the diff base (`git diff <base> -- <file>`) and that the change
> includes mutable operator sites (return values, comparison
> operators, match arms — not just whitespace, comments, or rustdoc).

---

## Files affected

| Path | Change | Lines (approx., before fix) |
|---|---|---|
| `xtask/src/mutants.rs` | Capture subprocess exit status; add `read_outcomes_or_short_circuit`; add `finalise_zero_mutant_run`; route `run()` through them; add 4 unit tests. | L138 (modify), L141-148 (modify), insert ~30 lines after L585, insert ~25 lines after that, insert ~50 lines of tests after L1128. |
| `.claude/rules/testing.md` | Add one paragraph under § "Mutation testing (cargo-mutants)" → "Per-step vs per-PR scoping" documenting empty-intersection behaviour. | ~10 lines inserted. |

No other call sites depend on `parse_outcomes` or `invoke_cargo_mutants`
directly; both are private to this module (verified — neither is
`pub`). The behavioural change is contained.

---

## Risk assessment

### Does the fix weaken the kill-rate gate?

**No.** The fix only adds a new branch for the case where `total_mutants
== 0` (no mutants in scope). The existing `evaluate_diff_gate` already
returns `GateStatus::Pass` for this case — the test
`diff_gate_passes_when_no_mutants_in_scope` (L946-961) asserts it.
The fix routes the synthetic-zero-mutant report through the same
gate, producing identical behaviour to a real cargo-mutants run that
generated zero mutants.

**Specifically NOT relaxed:**
- The "all-unviable" gate (`evaluate_diff_gate` at L631-647) still
  fails. It triggers on `total_mutants > 0 && caught == 0 && missed
  == 0`, which the synthetic report (`total_mutants: 0`) cannot
  produce.
- The kill-rate floor (80% diff, 60% workspace) is still enforced
  for any run that produces ≥1 caught-or-missed mutant.
- The workspace baseline drift gate is unchanged.

### Does the fix mask genuine subprocess crashes?

**No.** The discriminator (`read_outcomes_or_short_circuit`) treats
`absent file + non-zero exit` as a hard error and bails with a
sharper message ("subprocess likely crashed"). The only state newly
treated as success is `absent file + exit 0`, which is unambiguously
the cargo-mutants short-circuit path — a normal completed mutation
loop *always* writes `outcomes.json` before returning 0, by
upstream contract.

If cargo-mutants ever changes to write `outcomes.json` on the
empty-intersection path (a more user-friendly upstream change), the
wrapper continues to work — `parse_outcomes` parses the new report,
the gate sees `total_mutants == 0`, and the existing `Pass` branch
fires. The fix is forward-compatible.

### Does it affect other call sites?

`run()`, `invoke_cargo_mutants`, and `parse_outcomes` are all
module-private. The xtask CLI invokes `run()` and inspects only its
`Result<()>`. No other crate depends on these symbols (confirmed —
the only external surface in `xtask/src/main.rs` is the
`Mutants(...)` subcommand variant, which calls `mutants::run(...)`).

### Does it affect CI?

CI (`.github/workflows/...` mutants jobs) parses
`target/xtask/mutants-summary.json`. After the fix, the file is now
written even on the zero-mutant path (it currently is not, because
the wrapper bails before `write_summary`). This is an *improvement*:
CI no longer needs to defensively handle a missing summary file when
the diff happened to produce zero mutants. No CI workflow breakage
is expected; any consumer reading `status` will see `"pass"` and
proceed.

### Does it affect the per-step DELIVER inner-loop discipline?

**Improves it.** The rules currently recommend per-step `--file`
runs (per `.claude/rules/testing.md` § "Per-step vs per-PR
scoping"). Today, a step that touches a file with no diff-mutable
sites crashes the wrapper, which is the failure mode that prompted
this RCA. After the fix, such steps cleanly pass — matching the
documented "skip the mutation run for that step entirely" guidance
without forcing the developer to manually skip.

### Backwards-compatibility with the cargo-mutants schema

The synthetic `RawReport` uses only fields already present in the
struct; the `cargo_mutants_version: String::new()` empty-string
fallback is acceptable because the field is `#[serde(default)]` and
is surfaced in the summary as informational metadata only. If this
proves a CI consumer concern, the version can be probed via `cargo
mutants --version` in a follow-up — out of scope for this fix.

---

## Verification plan

After applying the fix:

1. **Reproduce the original crash locally** — run
   `cargo xtask mutants --diff origin/main --package xtask --file
   xtask/src/dst_lint.rs`. Pre-fix: bails with "no outcomes.json".
   Post-fix: prints `mutants: mode=diff total=0 — no mutants in
   scope … vacuous pass`, writes `target/xtask/mutants-summary.json`
   with `status="pass"`, exits 0.

2. **Confirm the populated-report path is unchanged** — run
   `cargo xtask mutants --diff origin/main --package overdrive-core`
   (or any package known to have mutable diff). Pre-fix and post-fix
   behaviour must be identical (kill rate, gate verdict, summary
   shape).

3. **Confirm the crash-detection path is preserved** — run the new
   `read_outcomes_bails_when_file_absent_and_exit_nonzero` unit test;
   assert it reproduces the original bail message shape for genuine
   crashes (absent file, non-zero exit).

4. **Inspect summary on the empty-intersection run** — `jq '.' <
   target/xtask/mutants-summary.json` must show
   `{"mode":"diff","total_mutants":0,"caught":0,"missed":0,
   "kill_rate_pct":100.0,"status":"pass",...}`.

5. **Run the full `xtask` test suite** —
   `cargo nextest run -p xtask --lib` must pass with the four new
   tests added.
