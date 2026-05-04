# Root Cause Analysis — cargo-mutants CI Job 30-min Timeout

**Branch:** `marcus-sa/reconciler-memory-redb`
**Job:** `mutants-diff` in `.github/workflows/ci.yml`
**Symptom:** Job killed at the 30-minute `timeout-minutes: 30` cap after running ~3 of 78 mutants
**Date:** 2026-05-04

---

## 1. Problem Definition and Scope

### Observable signal

```
Found 78 mutants to test
INFO Auto-set test timeout to 749s
ok       Unmutated baseline in 204s build + 149s test
MISSED   ...handlers.rs:634:9: ... in 10s build + 148s test
MISSED   ...reconciler_runtime.rs:259:9: ... in 11s build + 148s test
MISSED   ...reconciler_runtime.rs:499:9: ... in 11s build + 150s test
Error: The operation was canceled.
```

### Symptom math (validates the math given by the requester)

- Per-mutant wall: build (~10s, incremental) + test (~148s) = **~158s**
- Wall budget after baseline: 1800s (job cap) − 353s (baseline build+test) = **1447s**
- Mutants completable inside cap: 1447 / 158 ≈ **9 mutants**
- Mutants required: **78** → would need **~12,300s ≈ 205 minutes**

So the job is mathematically guaranteed to time out on this branch — the "cancelled" line is GHA SIGKILL at the 30-min cap, not an internal cargo-mutants failure. This is the symptom shape, not yet the root cause.

### Scope of investigation

In scope: the `mutants-diff` GHA job, the `cargo xtask mutants` wrapper at
`xtask/src/mutants.rs`, `.cargo/mutants.toml`, the `mutants` nextest profile
in `.config/nextest.toml`, and the per-PR diff size on this branch.

Out of scope: cargo-mutants internals beyond observed CLI behaviour;
the full-workspace nightly mutants run (different job, different cap);
the rest of the CI graph.

### Initial evidence collected

| Artifact | Evidence |
|---|---|
| `git diff origin/main --stat` | 107 files changed, 12,258 insertions, 1,502 deletions, 18 commits |
| `git diff origin/main -- 'crates/*/src/'` | 20 source files, ~3,300 insertions across `overdrive-control-plane`, `overdrive-core`, `overdrive-sim` |
| `xtask/src/mutants.rs::build_cargo_mutants_args` | `--test-workspace=false` is gated on `!scope.packages.is_empty() && !scope.test_whole_workspace` |
| `.github/workflows/ci.yml` lines 543-558 | CI calls `cargo xtask mutants --diff origin/main --features integration-tests` — **no `--package`**, **no `--file`** |
| `.github/workflows/ci.yml` line 497 | `timeout-minutes: 30` |
| `.config/nextest.toml` `[profile.mutants]` | Excludes only trybuild binaries; integration entrypoints still run |
| `.cargo/mutants.toml` | Excludes `xtask/**`, `**/tests/**`, generated paths, `Default::default` synth bodies, `unsafe`, `select!` etc. |
| `mutants-baseline/main/kill_rate.txt` | `100.0` (workspace baseline; not consulted by `--diff` mode) |

Per-mutant test wall is ~148s — the same as the unmutated baseline test wall (149s). cargo-mutants' auto-timeout (749s ≈ 5× baseline) is not even close to firing; tests are not slow on a per-test basis, the **suite is** slow because the suite is large.

---

## 2. Toyota 5 Whys Analysis — Multi-Branch

Five branches investigated in parallel. Each follows the symptom chain through five levels with verifiable evidence.

```
PROBLEM: cargo-mutants CI job times out at 30 min. 78 mutants × ~158s per
         mutant ≈ 205 minutes required, ~9 mutants fit in the budget.
```

---

### Branch A — Per-mutant test surface is the whole-workspace integration suite

**WHY 1A.** Each mutant takes ~148s of test time, identical to the unmutated baseline test wall.
[Evidence: `Unmutated baseline in 204s build + 149s test`; first three mutants `... in 10s build + 148s/148s/150s test` — variance under 2%]

**WHY 2A.** The per-mutant test invocation runs the **whole workspace** test suite, not a per-package subset.
[Evidence: `xtask/src/mutants.rs:315` — `if !scope.packages.is_empty() && !scope.test_whole_workspace { args.push("--test-workspace=false") }`. CI passes neither `--package` nor `--file` (`.github/workflows/ci.yml:558`); `Scope::default()` therefore yields `packages: []`; the `--test-workspace=false` arg is never appended; cargo-mutants defaults to `--test-workspace=true` (its documented behaviour) and reruns the full workspace per mutant.]

**WHY 3A.** The wrapper exposes `--package` / `--file` as scope narrowing flags, but the canonical CI invocation never uses them.
[Evidence: `xtask/src/main.rs:188` — `package: Vec<String>` is unused on the CI command line at `.github/workflows/ci.yml:558`. The xtask docstring at `xtask/src/mutants.rs:67-80` describes `Scope` as the narrowing surface but does not require it for diff mode.]

**WHY 4A.** The `--diff` flag and `--package` flag are independently optional, with no automatic inference: cargo-mutants generates mutants only from files in the diff (correct), but reruns tests **across the entire workspace** for every one of those mutants regardless of which crate the mutant lives in. The wrapper does not derive `--package` from the diff's owning crates.
[Evidence: `xtask/src/mutants.rs:288-297` — diff/workspace branching; mutually exclusive but no derivation. The diff on this branch hits 3 src crates (`overdrive-control-plane`, `overdrive-core`, `overdrive-sim`); a mutant in any one of those still runs `overdrive-store-local`'s snapshot proptest, `overdrive-scheduler`'s acceptance suite, and every other crate's tests — none of which can possibly catch a mutation in another crate's source.]

**WHY 5A — ROOT CAUSE A.** The per-PR mutation invocation pays full-workspace test wall-clock for every mutant, even though only the owning crate's tests can kill that mutant. The wrapper has the `--test-workspace=false` knob and documents it as "the single largest wall-clock win on a large workspace" (`xtask/src/mutants.rs:312-314`), but the CI command line never engages it because it is gated on `--package` being passed, and `--package` is not passed.

**SOLUTION A.** Either (a) CI passes `--package` for each crate touched in the diff and runs N parallel mutants jobs; or (b) the wrapper auto-derives `--package` from the diff's owning crates and engages `--test-workspace=false` automatically. Option (b) is the structural fix — see §4.

---

### Branch B — The `--features integration-tests` flag pulls the slow lane into per-mutant runs

**WHY 1B.** Per-mutant test wall is ~148s, dominated by integration tests that require the `integration-tests` feature flag.
[Evidence: `.github/workflows/ci.yml:558` passes `--features integration-tests`; the unmutated baseline test wall (149s) is consistent with running the workspace integration suite, which `.claude/rules/testing.md` §"Integration vs unit gating" explicitly characterises as the slow lane.]

**WHY 2B.** The `mutants` nextest profile excludes only trybuild binaries (`compile_pass`, `compile_fail`); it does not exclude `binary(integration)`.
[Evidence: `.config/nextest.toml:122-123` — `default-filter = '!(binary(compile_pass) | binary(compile_fail))'`. Integration entrypoints (`tests/integration.rs`) are NOT filtered out.]

**WHY 3B.** `.claude/rules/testing.md` §"Mutation testing (cargo-mutants) → Usage" explicitly mandates `--features integration-tests` on every mutation invocation. The reasoning given is that without it, "those tests don't compile — which means cargo-mutants runs the mutation against a build where the tests that would catch it are absent, and kill rate is silently understated."
[Evidence: `.claude/rules/testing.md` lines under "Why `--features integration-tests` is explicit, not auto-added".]

**WHY 4B.** The rule conflates two distinct concerns: (a) **compilation** of integration-test code (needed so mutations in that code can be evaluated; needed so trait-shape mutations are caught when they would break integration test code); (b) **execution** of every integration test on every mutant (which is what dominates wall-clock). cargo-mutants runs whatever nextest profile says to run; the current profile says "run everything that compiles." The rule was written for a smaller, slower-growing test surface and the budget-vs-coverage trade-off was never re-examined.

**WHY 5B — ROOT CAUSE B.** Per-PR mutation runs execute the entire integration-test surface on every mutant, even though most integration tests cannot possibly distinguish the mutated branch from the original (a mutation in `overdrive-control-plane::handlers::restart_budget_from_rows` cannot be killed by `overdrive-store-local`'s snapshot roundtrip proptest, `overdrive-sim`'s gossip test, or `overdrive-scheduler`'s placement test). Most of the 148s/mutant is testing things the mutant cannot affect.

**SOLUTION B.** Combine with Solution A: scoping by package via `--test-workspace=false` automatically restricts integration tests to those in the owning crate, dropping per-mutant time from ~148s to whatever the owning crate's own integration suite costs (typically 10-30s in this workspace).

---

### Branch C — The diff is genuinely large because of the redb migration

**WHY 1C.** 78 mutants is a lot for a per-PR run.
[Evidence: cargo-mutants found 78 viable mutants after `.cargo/mutants.toml` exclusions.]

**WHY 2C.** The diff against `origin/main` is 107 files, 12,258 insertions, 1,502 deletions, across 20 source files in 3 crates touched by mutation generation.
[Evidence: `git diff origin/main --stat | tail -3`; `git diff origin/main --stat -- 'crates/*/src/' | tail -10`.]

**WHY 3C.** The branch (`marcus-sa/reconciler-memory-redb`) is delivering ADR-0035 + ADR-0036 + ADR-0037 in one PR — a substantial architectural change that adds the `ViewStore` port, a redb host adapter, a sim adapter, the typed `TerminalCondition` enum, and rewrites both the `Reconciler` trait and `ReconcilerRuntime`.
[Evidence: 18 commits on the branch; commit titles include "feat(control-plane): add RedbViewStore host adapter", "refactor(core): collapse Reconciler trait", "feat(control-plane): wire ReconcilerRuntime to ViewStore"; three new ADRs in `docs/product/architecture/adr-0035-*.md` through `adr-0037-*.md`.]

**WHY 4C.** The repository's per-PR mutation budget assumes diffs roughly proportional to a single feature step (typically <20 src files, <500 lines), per `.claude/rules/testing.md` §"Per-step vs per-PR scoping" which recommends file-level scoping during the inner loop and suggests the unscoped per-package run "once before opening the PR (final check)." Branch-scope PRs that aggregate multiple architectural commits exceed that envelope.

**WHY 5C — ROOT CAUSE C.** The 30-minute job cap is sized for typical per-PR diffs at typical per-mutant wall-clock. When either factor doubles (this branch hits ~5× both), the cap is breached. The cap is not load-bearing in either direction — it was set defensively to prevent runaway runs, but the choice of 30 minutes was not derived from a wall-clock-per-mutant × expected-diff-size budget calculation.

**SOLUTION C.** The 30-minute cap is correct *as a guardrail against runaway runs* but wrong *as a budget for legitimate large PRs*. The real fix is Solution A/B (drop per-mutant time below the cap), not raising the cap. Raising the cap alone moves the failure mode from "30-min timeout" to "60-min timeout on the next refactor PR" without addressing the underlying scaling.

---

### Branch D — The wrapper does not parallelise

**WHY 1D.** All 78 mutants run sequentially on a single GHA runner.
[Evidence: cargo-mutants by default uses `--jobs 1`; the wrapper at `xtask/src/mutants.rs::build_cargo_mutants_args` does not set `--jobs`; `.github/workflows/ci.yml::mutants-diff` runs as a single job, not a matrix.]

**WHY 2D.** cargo-mutants supports `--jobs N` for parallel mutation evaluation; the wrapper does not expose it.
[Evidence: cargo-mutants documentation; `MutantsArgs` struct at `xtask/src/main.rs:154-210` exposes `file`, `package`, `features`, `test_whole_workspace`, `diff`, `workspace`, `baseline` — no `jobs` field.]

**WHY 3D.** Even with parallel mutants, GHA runners (`ubuntu-latest`) have 2 vCPUs; meaningful parallelism on a single runner is limited. Real parallelism needs a job matrix splitting mutants across runners.
[Evidence: GHA `ubuntu-latest` runner specs (2 vCPU, 7 GB RAM, public docs); `mutants-diff` job is a single `runs-on: ubuntu-latest`.]

**WHY 4D.** The mutation testing CI design treats the per-PR run as a single sequential job because the original budget calculation (~10-20 mutants × ~30-60s each = 5-20 min) made parallelism unnecessary. The redb-migration PR is the first run that reveals this assumption was load-bearing.

**WHY 5D — ROOT CAUSE D.** Sequential evaluation is fine for small diffs; it scales linearly with diff size and is the dominant sensitivity once the per-mutant wall-clock issue (Branches A/B) is fixed. Parallelism is the optimisation lever once the inner loop is right-sized.

**SOLUTION D.** Defer until A and B are fixed. With A+B in place, per-mutant time drops to ~30s and 78 mutants × 30s = 39 min — still over the cap. A 2-3 way matrix split (`mutants-diff (overdrive-control-plane)`, `mutants-diff (overdrive-core)`, `mutants-diff (overdrive-sim)`) parallelises the remaining wall and brings each leaf under the 30-min cap. This is the structural prevention strategy.

---

### Branch E — Build cost per mutant is also non-trivial, and `mold` does not help mid-run

**WHY 1E.** Each mutant pays ~10s of build time.
[Evidence: `MISSED ... in 10s build + 148s test` — recurring.]

**WHY 2E.** cargo-mutants creates a fresh scratch tree per mutant in `mutants.out/` and runs `cargo build` there. Incremental compilation is cargo's default, but the scratch tree is a new target dir per mutation by default.
[Evidence: `cargo-mutants --help` documents `--build-timeout`; per-mutant scratch tree is its standard mode of operation.]

**WHY 3E.** `mold` (the linker per `.cargo/config.toml`, recently merged on `main` in 2b439c6) reduces link time but does not eliminate compilation. The 10s/mutant is mostly rustc, not link.
[Evidence: 2b439c6 commit message — "build(linker): switch to mold on Linux for ~3x incremental rebuild"; the speedup applies to incremental builds within a single target dir, not to per-mutant scratch builds.]

**WHY 4E.** sccache is configured (lines 96-105 of `ci.yml`) but as a **soft-fail**: if the GHA cache backend is unreachable, RUSTC_WRAPPER is left unset. When sccache *is* working, per-mutant build cost falls dramatically; when it's not, builds run uncached. The job log shows sccache was either unavailable or cold — 10s rebuilds suggest partial cache hits.

**WHY 5E — ROOT CAUSE E.** Build cost is a secondary contributor (10s × 78 = 13 min of pure build, ~22% of theoretical wall). Fixing test wall (Branch A/B) makes build cost the next visible bottleneck, but on its own it does not breach the cap.

**SOLUTION E.** Ensure sccache is consistently warm for the mutants job; verify the `phase-1-mutants` rust-cache key isn't churning across runs. This is a P3 follow-up — not the critical path for this PR.

---

## 3. Cross-Branch Validation

### Multiplicative interaction

```
Total wall ≈ baseline_build (204s) + baseline_test (149s)
            + N_mutants × (per_mutant_build + per_mutant_test)
          = 353s + 78 × (10s + 148s)
          = 353s + 12,324s
          ≈ 12,677s ≈ 211 min
```

Cap is 1,800s (30 min). Required: 211 min. Deficit: 181 min.

### Each branch's contribution if fixed in isolation

| Fix in isolation | Per-mutant wall | Total wall | Result |
|---|---|---|---|
| Status quo | 158s | 211 min | TIMEOUT (×7 over cap) |
| Solution A only (`--test-workspace=false` per owning crate) | ~30s | 47 min | TIMEOUT (×1.6 over cap) |
| Solution B only (drop integration suite from per-mutant) | ~50s | 71 min | TIMEOUT (×2.4 over cap) |
| Solution A + B (package-scoped + drop full integration) | ~20s | 32 min | borderline; one slow PR away from breach |
| A + B + matrix split 3-way (Solution D) | ~20s, 26 mutants/leaf | 9-12 min/leaf | PASS comfortably |
| Raise cap to 60 min only (no other fix) | 158s | 211 min | still TIMEOUT |
| Raise cap to 240 min only | 158s | 211 min | PASS but at 4× compute spend, fragile |

Solutions A and B are required-and-sufficient for this PR. Solution D (matrix split) is the structural prevention.

### Backwards chain validation

Each WHY-5 traces forward to the symptom:

- A: full-workspace per-mutant → 148s/mutant test wall → 78 × 148s exceeds 30 min → TIMEOUT ✓
- B: integration-tests feature pulls full integration suite → contributes most of 148s → same path as A → TIMEOUT ✓
- C: large diff → 78 mutants → multiplies (A+B) → TIMEOUT ✓
- D: sequential evaluation → 78 mutants serialized → TIMEOUT ✓
- E: 10s build/mutant → 13 min cumulative → contributes to overrun but not sole cause ✓

No contradictions. A, B, C are mutually amplifying; D is the lever once A+B are fixed; E is a secondary contributor.

### Completeness check — could anything else be missing?

Considered and ruled out:

- **Diff-scoping is not working.** REJECTED. Wrapper materialises a real diff file at `target/xtask/mutants.diff` (line 233 of `mutants.rs`) and passes it via `--in-diff` (correct cargo-mutants spelling, per the wrapper's own docstring at line 53-55). The `Found 78 mutants to test` line confirms cargo-mutants only generated mutants from diff-touched files; if scoping were broken, the count would be in the thousands.
- **Integration-tests feature gate is broken on this branch.** REJECTED. The unmutated baseline runs in 149s; if integration tests were not compiling under the feature, baseline would error out before any mutant ran (the wrapper has an explicit "unmutated baseline failed" path at `mutants.rs:496-508`).
- **A specific test is hanging.** REJECTED. Per-mutant wall is consistent at 148-150s across the three observed mutants; a hanging test would exceed the auto-set 749s timeout and cargo-mutants would log `TIMEOUT` rather than `MISSED`.
- **GHA runner contention.** REJECTED. Per-mutant wall matches baseline test wall to within 1%; runner-side noise would produce >10% variance.

---

## 4. Solution Development

Each solution is mapped to one or more root causes. Solutions are labelled **immediate** (lands this PR; ships the redb migration) vs **structural** (prevents recurrence).

### S1 — IMMEDIATE: pass `--package` per owning crate via a job matrix

**Addresses:** Root Causes A, B, D
**Target:** ship this PR within the existing 30-min cap
**Effort:** ~15 min of YAML

Replace the single `mutants-diff` job with a matrix over the 3 crates touched by this branch's diff:

```yaml
mutants-diff:
  if: github.event_name == 'pull_request'
  timeout-minutes: 30
  strategy:
    fail-fast: false
    matrix:
      package:
        - overdrive-control-plane
        - overdrive-core
        - overdrive-sim
  steps:
    # ... existing setup ...
    - run: |
        sudo -E env "PATH=$PATH" cargo xtask mutants \
          --diff origin/main \
          --features integration-tests \
          --package ${{ matrix.package }}
```

This engages `--test-workspace=false` automatically (line 315 of `mutants.rs`), restricting per-mutant tests to the owning crate's suite. Matrix runs in parallel across 3 GHA runners; each leaf evaluates ~26 mutants × ~30-50s ≈ 13-22 min. Fits comfortably in the cap.

**Risk:** matrix listing is hand-maintained — adding a 4th mutated crate requires updating the matrix. Mitigated by S3 (auto-derive matrix from diff).

**Trade-off:** the matrix listing is hard-coded today; structurally we want to derive it. S3 makes this dynamic.

### S2 — IMMEDIATE alternative: tighten the `mutants` nextest profile to drop integration entrypoints

**Addresses:** Root Cause B
**Target:** mitigation if matrix split is rejected for this PR
**Effort:** 2-line change to `.config/nextest.toml`

```toml
[profile.mutants]
# Add binary(integration) to the exclusion. Trade-off documented inline.
default-filter = '!(binary(compile_pass) | binary(compile_fail) | binary(integration))'
```

Drops integration tests from per-mutant runs. Per-mutant test wall would fall from ~148s to whatever the unit + acceptance suite costs (likely 30-60s).

**Cost:** loses kill-rate signal for any mutation only catchable by an integration test. The redb-migration PR adds new integration tests at `tests/integration/redb_view_store.rs` and `tests/integration/redb_view_store_no_leak.rs` — those would no longer participate in mutation testing. This is a coverage regression, not just a wall-clock optimisation.

**Recommendation:** prefer S1 over S2. S2 is the fallback only if matrix YAML is rejected.

### S3 — STRUCTURAL: auto-derive `--package` from the diff in the wrapper

**Addresses:** Root Causes A, B, D
**Target:** prevent recurrence on every future large PR
**Effort:** ~50 LoC in `xtask/src/mutants.rs` + acceptance test

In `Mode::Diff` mode, after materialising the diff file, parse it for `crates/<crate>/src/` paths and derive a default package set. Engage `--test-workspace=false` automatically for diff-mode runs unless an opt-out flag is passed.

```rust
// Pseudocode in xtask::mutants
fn derive_packages_from_diff(diff_bytes: &[u8]) -> Vec<String> {
    // Walk diff hunks; collect distinct `crates/<crate>/src/...` entries.
    // Return Vec<String> of crate names.
}

// In invoke_cargo_mutants for Mode::Diff:
if scope.packages.is_empty() {
    let derived = derive_packages_from_diff(&diff_bytes);
    if !derived.is_empty() {
        // Either: add --package <derived> + --test-workspace=false,
        // or: split into matrix (requires CI cooperation).
    }
}
```

CI then becomes:

```yaml
- run: cargo xtask mutants --diff origin/main --features integration-tests
```

with no manual matrix to maintain. The wrapper handles the package scoping; CI optionally fans out via a generated matrix in a follow-up if mutant counts demand it.

**Risk:** if the wrapper auto-narrows to packages, mutations in code reachable only by tests in *another* crate would not be killed. The wrapper exposes `--test-whole-workspace` (already in `MutantsArgs`) as the opt-out for that case.

**Recommendation:** ship as a follow-up PR after the redb-migration PR lands. Land S1 first (unblocks the migration); land S3 second (prevents the next PR from hitting this).

### S4 — STRUCTURAL: matrix-split mutants-diff via dynamic matrix

**Addresses:** Root Cause D
**Target:** the case where one crate's diff is itself large enough to time out
**Effort:** ~30 LoC YAML + S3 dependency

GHA supports dynamic matrices via `jobs.<id>.outputs`. A pre-job parses the diff, emits a JSON list of `{package, files}`, and the matrix consumes it:

```yaml
mutants-discover:
  outputs:
    matrix: ${{ steps.compute.outputs.matrix }}
  steps:
    - id: compute
      run: |
        echo "matrix=$(cargo xtask mutants-discover --diff origin/main)" >> "$GITHUB_OUTPUT"

mutants-diff:
  needs: mutants-discover
  strategy:
    matrix: ${{ fromJSON(needs.mutants-discover.outputs.matrix) }}
```

`cargo xtask mutants-discover` is a new subcommand that emits the JSON; reuses the diff-parsing logic from S3.

**Recommendation:** P2 — defer until S3 lands and a future PR proves S3 alone insufficient.

### S5 — REJECTED: raise the 30-minute cap

Symptom-only fix. Without S1/S3, the next 100-file PR breaches a 60-min cap; the next 200-file PR breaches a 120-min cap. Compute spend grows linearly with diff size; failure mode is preserved. Documented for completeness; do not adopt.

### S6 — EARLY DETECTION: emit per-mutant wall-clock to step summary

**Addresses:** observability gap
**Effort:** ~20 LoC in the failure-annotation step at `ci.yml:572-606`

When the job times out (rather than fails the gate), parse `mutants.out/log/*` for build/test wall-clock per completed mutant and emit a summary table:

```
Per-mutant wall-clock (last N completed):
  handlers.rs:634   10s build + 148s test = 158s
  reconciler_runtime.rs:259  11s build + 148s test = 159s
At this rate, 78 mutants would need ~205 min vs. 30-min cap.
```

This makes the failure mode self-diagnosing — a future engineer hitting this sees the math in the GHA summary and knows to reach for `--package`.

**Recommendation:** P2; ship alongside S3.

---

## 5. Prevention Strategy and Prioritisation

### Recommended sequencing

| Order | Solution | Addresses | Effort | Ships in |
|---|---|---|---|---|
| 1 | S1 — matrix-split with hand-listed packages | A, B, D | 15 min | this PR (unblocks redb migration) |
| 2 | S3 — auto-derive `--package` in wrapper | A, B, D structurally | ~2h | follow-up PR |
| 3 | S6 — per-mutant wall-clock in step summary | observability | ~30 min | same follow-up PR as S3 |
| 4 | S4 — dynamic matrix from `mutants-discover` | D, future scaling | ~1h | only if S3 proves insufficient |

### Smallest change to ship this PR

S1 alone: add `strategy.matrix.package` to the `mutants-diff` job in `.github/workflows/ci.yml`. Three packages: `overdrive-control-plane`, `overdrive-core`, `overdrive-sim`. Each leaf takes the existing per-mutant invocation and adds `--package ${{ matrix.package }}`. No code changes; YAML-only.

### Recurrence prevention

The structural problem — wrapper offers `--test-workspace=false` only when `--package` is explicitly given — is what made this branch the canary. S3 inverts that default for diff mode (auto-derive), so future large PRs hit the right scoping by default. The 30-min cap stays as a safety net; if S3+matrix still timeout, S6's annotations name the cause clearly enough that the next engineer knows what to do.

### What this RCA also reveals

- **`.claude/rules/testing.md` §"Per-step vs per-PR scoping" assumes file-scoped `--file` flags during the inner loop**, with the per-package run "once before opening the PR." For PRs that span many files, the per-PR run already times out under the rule's own assumptions — the rule documents this only obliquely. After S3 lands, the rule should be updated to reflect that diff-mode auto-derives packages and matrix-splits as needed.
- **The `mutants-diff` job's hard-coded `if: github.event_name == 'pull_request'` is correct** — diff scoping has nothing meaningful to do on `push: main`. Confirmed; not a contributor.
- **The workspace mutants nightly job (`nightly.yml`) does not have this problem** because (a) it's nightly and the runner has a longer timeout, (b) full-workspace is the intent there. The two paths diverge correctly.

---

## 6. Causal Chain Summary

```
ROOT CAUSE A: per-mutant tests run the whole workspace because
              --test-workspace=false is gated on --package and CI
              passes neither --package nor --file
   ↓
ROOT CAUSE B: --features integration-tests amplifies A by pulling
              the full integration suite into per-mutant test wall
   ↓
ROOT CAUSE C: redb-migration branch has 18 commits / 107 files /
              3 crates of source change → 78 mutants exceeds the
              budget A+B was designed against
   ↓
ROOT CAUSE D: sequential evaluation amplifies (A × B × C) linearly
   ↓
SYMPTOM:      30-min GHA timeout after ~3 of 78 mutants
```

Fixing (A+B) drops per-mutant time ~5×; fixing (D) parallelises the residual; (C) becomes a non-issue when both upstream causes are addressed.
