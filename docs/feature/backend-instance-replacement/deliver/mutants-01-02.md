# Mutation evidence — step 01-02 (generation gate + current-instance veto + R5 guard)

**Step**: `01-02` — Desired-run generation precursor + current-instance-scoped
reconciler veto + (review-01-02 BLOCKER-1) the R5 draining-instance wait-guard.
**Surface under gate**: `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs`
**Roadmap gate**: kill-rate ≥ 80% (`roadmap.json` step `01-02`, final criterion)
with FIVE named kill targets; this evidence adds a SIXTH target — the R5 guard
landed by the BLOCKER-1 fix (fresh mutable surface that must also be covered).
**Code under test SHA**: `b228982d` (the BLOCKER-1 fix: R5 guard + strengthened
S-BIR-STOP-ONCE + the (a)/(b) test improvements) on top of `3537b066` (the
original 01-02 reconciler surface).
**Closes**: review-01-02 BLOCKER-2 ("the mandatory mutation gate for 01-02 has
no recorded evidence").

---

## TL;DR

- **Diff-scoped gate (the CI-enforced scope, `--diff origin/main`): 12 mutants,
  12 caught, 0 missed — kill-rate 100 %, PASS.** This is the scope CI runs on
  every PR; it covers every line the `backend-instance-replacement` branch
  changed (the 01-02 reconciler surface + the new R5 guard).
- **Whole-file gate (broader signal, `--workspace --package --file`): 62 mutants,
  57 caught, 2 missed, 3 unviable — kill-rate 96.6 %, PASS** (≥ 80 % bar). The 2
  misses are in **pre-existing helper code untouched by this feature branch**
  (`node_free_capacity`, `classify_natural_exit_terminal`) — neither is in the
  branch diff, neither is one of the six mandatory targets. Detail in § 3.
- **Five of the six mandatory targets are killed by tool-generated mutants**
  (verbatim caught list in § 2). The sixth — the stamp `observed = desired →
  observed + 1` — is a **cargo-mutants literal-assignment blind spot** (the same
  class as 01-01's `+1 → +0`, project memory
  `reference_cargo_mutants_blind_to_spawn_blocking_and_saturating_add`); it is
  proven by an **executed manual mutation proof** (§ 4): the hand-applied
  `= observed + 1` makes S-BIR-COALESCE-PLACE FAIL (`left: 1, right: 2`).

---

## 1. The recorded gate commands + verbatim results

Both runs go through Lima (`--features integration-tests` requires it) and read
the **guest-side** summary (on macOS Lima the summary lands in the guest target
dir, not host `target/xtask/mutants-summary.json` — project memory
`reference_lima_mutation_summary_host_path_trap`).

### 1a. Diff-scoped (CI gate)

```
cargo xtask lima run -- cargo xtask mutants --diff origin/main \
  --features integration-tests --package overdrive-core \
  --file crates/overdrive-core/src/reconcilers/workload_lifecycle.rs
```

Verbatim (captured at HEAD `b228982d`):

```
Found 12 mutants to test
ok      Unmutated baseline in 17s build + 0s test
 INFO Auto-set test timeout to 20s
12 mutants tested in 77s: 12 caught
mutants: mode=diff total=12 caught=12 missed=0 timeout=0 unviable=0 kill_rate=100.0%
mutants: PASS
```

Guest `mutants-summary.json`:

```json
{ "mode": "diff", "cargo_mutants_version": "27.1.0", "total_mutants": 12,
  "caught": 12, "missed": 0, "timeout": 0, "unviable": 0,
  "kill_rate_pct": 100.0, "base_ref": "origin/main", "status": "pass" }
```

The 01-01 `--diff --file` vacuous-pass trap (`total_mutants=0`) does **NOT** recur
here: the R5 guard added by the BLOCKER-1 fix and the 01-02 reconciler surface are
both fresh diff lines carrying mutable-operator sites, so the diff scope is
non-vacuous (12 mutants) and meaningful.

### 1b. Whole-file (broader signal)

```
cargo xtask lima run -- cargo xtask mutants --workspace \
  --features integration-tests --package overdrive-core \
  --file crates/overdrive-core/src/reconcilers/workload_lifecycle.rs
```

Verbatim:

```
Found 62 mutants to test
ok      Unmutated baseline in 14s build + 0s test
 INFO Auto-set test timeout to 20s
MISSED  ...workload_lifecycle.rs:935:43: replace == with != in node_free_capacity in 3s build + 0s test
MISSED  ...workload_lifecycle.rs:1166:9: replace && with || in classify_natural_exit_terminal in 6s build + 0s test
62 mutants tested in 6m: 2 missed, 57 caught, 3 unviable
mutants: mode=workspace total=62 caught=57 missed=2 timeout=0 unviable=3 kill_rate=96.6%
mutants: baseline=100.0% drift=-3.4pp
mutants: WARN — mutants drift -3.4pp below baseline 100.0% (current=96.6%)
```

The `WARN` is the soft drift signal (`.claude/rules/testing.md`: a drop > 2pp
soft-warns); the absolute kill-rate 96.6 % is comfortably above the ≥ 80 % gate,
and the diff-scoped CI gate (§ 1a) is a clean 100 %.

---

## 2. The caught mutants for the mandatory targets (verbatim from `caught.txt`)

The full whole-file caught list is 57 mutants; the lines load-bearing for the six
mandatory targets, verbatim:

```
# Target #5 — the `observed < desired` restart_pending comparison (line 491):
workload_lifecycle.rs:491:64: replace < with == in WorkloadLifecycle::reconcile_inner
workload_lifecycle.rs:491:64: replace < with >  in WorkloadLifecycle::reconcile_inner
workload_lifecycle.rs:491:64: replace < with <= in WorkloadLifecycle::reconcile_inner

# Target #6 — the NEW R5 guard `restart_pending && current_alloc(...).is_some_and(Draining)` (line 540):
workload_lifecycle.rs:540:21: replace && with || in WorkloadLifecycle::reconcile_inner
workload_lifecycle.rs:540:75: replace == with != in WorkloadLifecycle::reconcile_inner

# Targets #1 + #4 — the scoped veto `!restart_pending && current_alloc(...is_operator_stopped)` (line 601):
workload_lifecycle.rs:601:37: replace && with || in WorkloadLifecycle::reconcile_inner
workload_lifecycle.rs:601:20: delete ! in WorkloadLifecycle::reconcile_inner
workload_lifecycle.rs:1106:5:  replace is_operator_stopped -> bool with true
workload_lifecycle.rs:1106:5:  replace is_operator_stopped -> bool with false

# Target #3 — current_alloc numeric-vs-lexical selection (lines 981, 1012):
workload_lifecycle.rs:981:5:   replace alloc_attempt_index -> Option<u32> with None
workload_lifecycle.rs:981:5:   replace alloc_attempt_index -> Option<u32> with Some(0)
workload_lifecycle.rs:981:5:   replace alloc_attempt_index -> Option<u32> with Some(1)
workload_lifecycle.rs:1012:5:  replace current_alloc -> Option<&'a AllocStatusRow> with None
```

(Target #2 — the stamp at line 891 — has **no** entry in `caught.txt`, `missed.txt`,
or `unviable.txt`: cargo-mutants generates no mutant for it. See § 4.)

---

## 3. The two whole-file misses are pre-existing, out-of-scope, and not mandatory

```
workload_lifecycle.rs:935:43: replace == with != in node_free_capacity
workload_lifecycle.rs:1166:9: replace && with || in classify_natural_exit_terminal
```

- `node_free_capacity` (line 935) is the `first_fit_place` capacity helper; the
  `==` at col 43 is the `alloc.state == AllocState::Running` predicate inside a
  `.filter()` count. Mutating it to `!=` counts non-Running allocs as occupying
  capacity — the suite's single-node fixtures never exercise enough concurrent
  allocs for the capacity arithmetic to flip a placement decision, so no test
  observes the change.
- `classify_natural_exit_terminal` (line 1166) is the Job-kind natural-exit
  classifier (ADR-0037); the `&&` at col 9 is in the `Terminated && reason ==
  Stopped{Process}` clean-exit discriminator.

Both functions are **pre-existing code untouched by this feature branch** —
confirmed by `git diff origin/main -- workload_lifecycle.rs` (neither `fn
node_free_capacity` nor `fn classify_natural_exit_terminal` appears in the diff),
and by the diff-scoped run finding **zero** mutants on either line (it found 12,
all in 01-02 surface). Neither is one of the six mandatory targets. They are out
of this step's mutable surface; the CI-enforced diff-scoped gate is a clean 100 %.

---

## 4. Manual mutation proof of the stamp behavior (mandatory target #2 — tool blind spot)

The roadmap names target #2 as the stamp flip `observed = desired → observed + 1`.
The production line is:

```rust
// crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:891
next_view.observed_generation = desired.generation;
```

cargo-mutants 27.1.0 generates **no mutant** here: this is a field-assignment of a
field-read (`= desired.generation`), and the tool has no operator that rewrites it
to `= self.observed_generation + 1`. (The same literal-assignment / arithmetic
blind spot 01-01 hit on `saturating_add(1) → saturating_add(0)`; project memory
`reference_cargo_mutants_blind_to_spawn_blocking_and_saturating_add`.) Confirmed:
no line-891 entry in `caught.txt` / `missed.txt` / `unviable.txt`.

Per the reviewer-sanctioned manual-proof procedure (mirrors `mutants-01-01.md`
§ 3), the mutation was applied **by hand**, the discriminating acceptance test was
run under Lima, and the source was reverted.

Mutation: `workload_lifecycle.rs:891`
`next_view.observed_generation = desired.generation`
→ `next_view.observed_generation = next_view.observed_generation + 1`.

Run:
`cargo xtask lima run -- cargo nextest run -p overdrive-core --features
integration-tests -E 'test(s_bir_coalesce_place...) | test(s_bir_coalesce_no_replay...)
| test(s_bir_restart_stopped...)'`

Result: **S-BIR-COALESCE-PLACE FAILED** — the mutation is decisively killed:

```
thread '...s_bir_coalesce_place_one_instance_stamps_to_latest_generation' panicked
  at crates/overdrive-core/tests/acceptance/workload_lifecycle_restart.rs:441:5:
assertion `left == right` failed: stamp is observed = desired (= 2), NOT observed + 1
  left: 1
 right: 2
```

The fixture is `desired.generation = 2`, `observed = 0`: the correct `= desired`
stamps `2`; the mutated `= observed + 1` stamps `0 + 1 = 1`, failing the
`observed == 2` assertion. (S-BIR-COALESCE-NO-REPLAY and S-BIR-RESTART-STOPPED
pass under the mutation because at `observed == desired == 2` / single-restart
`observed == 1` the two formulas coincide — COALESCE-PLACE is the discriminating
test, exactly as designed for the level-triggered coalesce.)

Source reverted immediately after; `git diff -- workload_lifecycle.rs` is empty.
The mutation was **never committed**.

---

## 5. Conclusion — every mandatory target accounted for

| # | Mandatory kill target | Killed by | Evidence |
|---|---|---|---|
| 1 | veto `any(is_operator_stopped)` → `current_alloc(...)` scoping | S-BIR-REGRESSION-STOPPED / -RUNNING, S-BIR-BUG3-PRESERVED | `601:37 &&→\|\|`, `601:20 delete !`, `1106:5 →false` caught (§ 2) |
| 2 | stamp `observed = desired → observed + 1` | S-BIR-COALESCE-PLACE | **manual proof** — COALESCE-PLACE FAIL `left:1 right:2` (§ 4); tool blind spot |
| 3 | `current_alloc` lexical-max flip | S-BIR-REGRESSION-NUMERIC + new `current_alloc_tests` direct proptest | `981` (3 variants), `1012 →None` caught (§ 2) |
| 4 | scoped-veto-never-fires | S-BIR-BUG3-PRESERVED | `601:20 delete !`, `1106:5 →false` caught (§ 2) |
| 5 | `observed < desired` comparison flip (`<`→`<=`/`==`/`>`) | S-BIR-SEQUENTIAL | `491:64` all 3 variants caught (§ 2) |
| 6 | **NEW R5 guard** (`restart_pending && current_alloc(...).is_some_and(Draining)`) | strengthened S-BIR-STOP-ONCE (`actions.is_empty()`) | `540:21 &&→\|\|`, `540:75 ==→!=` caught (§ 2) |

The mandatory mutation gate is now evidenced: the CI-enforced diff scope is a
clean 100 % (12/12), the whole-file scope is 96.6 % (57/62, the 2 misses
pre-existing and out-of-scope), and all six targets — including the BLOCKER-1 R5
guard and the tool-blind stamp — are demonstrably killed.
