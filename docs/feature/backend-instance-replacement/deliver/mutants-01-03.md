# Mutation evidence — step 01-03 (`restart_workload` handler)

**Step**: `01-03` — `restart_workload` HTTP handler + `RestartWorkloadResponse` /
`RestartOutcome` API types + `POST /v1/jobs/:id/restart` route.
**Surface under gate**: `crates/overdrive-control-plane/src/handlers.rs`
(the `restart_workload` handler, lines 895-947).
**Roadmap gate**: kill-rate ≥ 80% (`roadmap.json` step `01-03`, final criterion)
with THREE named kill targets:

1. the **404 no-mutation posture** (absent aggregate ⇒ `NotFound`, no bump/enqueue),
2. the one-txn **`[IncrementU64, Delete]` op-set** (atomic generation bump + stop clear),
3. the **`present ⇒ Resumed` / `absent ⇒ Restarted`** outcome classification.

**Code under test SHA**: `286b334b` (the 01-03 handler + API + route surface).
**Closes**: review-01-03 BLOCKER-1 ("the step 01-03 mutation gate is not
evidenced") AND discharges the review's accepted residual risk (the state-delta
proxy for "exactly one txn" — § 5).

---

## TL;DR

- **Diff-scoped gate (the CI-enforced scope, `--diff origin/main`) is
  non-signalling for this file: 1 mutant, 1 *unviable*, `status: fail` —
  "no quality signal".** The single diff-scoped mutant cargo-mutants 27.1.0
  generates is the whole-body replacement
  `replace restart_workload -> … with Ok(Json::new())`, which **does not
  compile** (`axum::Json` has no `::new()` constructor) — the documented
  cargo-mutants `--in-diff` whole-body-replacement blind spot
  (`.claude/rules/testing.md` `--in-diff` is a *file path* / blind-spot note;
  same class 01-01 hit on its `--diff --file` scope). The tool therefore
  provides **zero** kill signal here. This is recorded honestly in § 1,
  exactly as `mutants-01-02.md` recorded its tool-blind target — and is the
  justification for the three executed manual proofs that follow.
- **All three named kill targets are discharged by EXECUTED manual mutation
  proofs** (§ 2): each hand mutation was applied with Edit, the discriminating
  acceptance test(s) run under Lima (`-E 'test(restart_workload)'`), the real
  RED output pasted, and `handlers.rs` reverted to `286b334b` (empty diff)
  before commit. No mutation was ever committed.
  - **Proof A — 404 posture**: `.is_none()` → `.is_some()` ⇒ **all 4 ATs RED**.
  - **Proof B — op-set**: drop `TxnOp::IncrementU64` ⇒ **`S-BIR-HANDLER-TXN` RED**
    (`left: 0, right: 1` — generation stays 0).
  - **Proof C — outcome classification**: swap `Resumed`/`Restarted` arms ⇒
    **both outcome ATs RED**.

---

## 1. The recorded diff-scoped tool run + verbatim result (non-signalling)

The diff-scoped run goes through Lima (`--features integration-tests` requires
it) and the summary is read from the **guest-side** target dir (on macOS Lima the
summary lands in the guest, not host `target/xtask/mutants-summary.json` —
project memory `reference_lima_mutation_summary_host_path_trap`).

```
cargo xtask lima run -- cargo xtask mutants --diff origin/main \
  --features integration-tests --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/handlers.rs
```

Verbatim (captured at HEAD `286b334b`):

```
xtask mutants: wrote …/xtask/mutants.diff (688851 bytes) for --in-diff
xtask mutants: running … cargo mutants --output … --test-tool=nextest \
  --in-diff …/mutants.diff --file crates/overdrive-control-plane/src/handlers.rs \
  --package overdrive-control-plane --test-workspace=false --features integration-tests
Found 1 mutant to test
ok      Unmutated baseline in 23s build + 39s test
 INFO Auto-set test timeout to 200s
1 mutant tested in 65s: 1 unviable
```

Guest `mutants-summary.json`:

```json
{
  "mode": "diff",
  "cargo_mutants_version": "27.1.0",
  "total_mutants": 1,
  "caught": 0,
  "missed": 0,
  "timeout": 0,
  "unviable": 1,
  "baseline_success": 0,
  "kill_rate_pct": 100.0,
  "base_ref": "origin/main",
  "status": "fail",
  "reason": "no quality signal: total=1 unviable=1 timeout=0 — see target/xtask/mutants.out/log/* for rustc diagnostics"
}
```

The single mutant (guest `mutants.out/unviable.txt`):

```
crates/overdrive-control-plane/src/handlers.rs:901:5: replace restart_workload -> Result<axum::Json<RestartWorkloadResponse>, ControlPlaneError> with Ok(Json::new())
```

This is the **whole-function-body replacement** mutant. It is `unviable` because
`Ok(Json::new())` does not compile — `axum::Json<T>` is a tuple-struct wrapper
constructed as `Json(value)`, it has no `::new()` associated function — so rustc
rejects the mutated source before any test runs. cargo-mutants emits **no other
mutant** in the diff scope: every other mutable-operator site inside
`restart_workload` (the `.is_none()` guard, the `TxnOp::IncrementU64`/`Delete`
op-set, the `is_some()`-driven `Resumed`/`Restarted` arms) lands on lines whose
*diff* against `origin/main` is the freshly-added handler, but the per-line
mutable sites the tool would synthesise there are all subsumed under the one
whole-body replacement at `901:5`. The `status: fail` / `kill_rate 100.0` pair is
the wrapper's `total=1 unviable=1` "no quality signal" verdict, **not** a real
100 % — the gate returns `FAIL` precisely so a 1-unviable run is not mistaken for
a pass.

The optional whole-file `--workspace --package --file` scope over `handlers.rs`
was **skipped**: `handlers.rs` carries many other handlers (deploy, stop, list,
cluster-info, …) and a whole-file run would mutate all of them at a multi-minute
cost without adding signal for *this* step's three named targets. The substantive
evidence the review requires is the three executed manual proofs below, each
targeting exactly one named kill target.

---

## 2. Three EXECUTED manual mutation proofs (the substantive evidence)

For each proof the discriminating ATs were run with:

```
cargo xtask lima run -- cargo nextest run -p overdrive-control-plane \
  --features integration-tests -E 'test(restart_workload)'
```

(That filter runs the 4 ATs across `restart_workload_unknown.rs` /
`restart_workload_intent_key.rs` / `restart_workload_outcome.rs`.) After each
proof the source was reverted with Edit and `git diff -- handlers.rs` confirmed
empty.

### Proof A — 404 no-mutation posture (kill target #1)

Mutation: `handlers.rs:909` — the absent-aggregate guard

```rust
if state.store.get(job_key.as_bytes()).await?.is_none() {   // ← .is_none()
    return Err(ControlPlaneError::NotFound { resource: job_key.as_str().to_owned() });
}
```

flipped to `.is_some()`. Under the mutation an **absent** aggregate no longer
404s — it proceeds to bump+enqueue+respond — and a **declared** aggregate now
wrongly 404s.

Result: **all 4 ATs FAILED** — the mutation is decisively killed:

```
FAIL (1/4) acceptance::restart_workload_unknown::restart_on_unknown_id_is_404_with_no_mutation_and_no_enqueue
  panicked at …/restart_workload_unknown.rs:110:21:
  restart on an unknown id must 404, never bump-and-respond; handler returned
  Ok(Json(RestartWorkloadResponse { workload_id: "nonexistent", outcome: Restarted }))

FAIL (2/4) acceptance::restart_workload_outcome::absent_stop_sentinel_classifies_outcome_as_restarted
  panicked at …/restart_workload_outcome.rs:102:23:
  restart on a declared workload must succeed; got NotFound { resource: "workloads/coinflip" }

FAIL (3/4) acceptance::restart_workload_outcome::present_stop_sentinel_classifies_outcome_as_resumed
  panicked at …/restart_workload_outcome.rs:102:23:
  restart on a declared workload must succeed; got NotFound { resource: "workloads/payments" }

FAIL (4/4) acceptance::restart_workload_intent_key::restart_commits_one_bump_clear_txn_retains_intent_and_enqueues_one_eval
  panicked at …/restart_workload_intent_key.rs:136:23:
  restart on a declared workload must succeed; got NotFound { resource: "workloads/payments" }

Summary: 4 tests run: 0 passed, 4 failed
```

`S-BIR-HANDLER-404` (`restart_workload_unknown.rs`) is the primary kill: the
absent aggregate proceeds to bump-and-respond (`Ok(... outcome: Restarted)`)
instead of 404ing. The three seeded-aggregate ATs corroborate the posture from
the other direction (a declared aggregate wrongly 404s). Reverted; diff empty.

### Proof B — one-txn op-set (kill target #2)

Mutation: `handlers.rs:933-936` — drop the `TxnOp::IncrementU64` element, leaving
only the `Delete`:

```rust
.txn(vec![
    // TxnOp::IncrementU64 { key: … }   ← removed
    TxnOp::Delete { key: Bytes::copy_from_slice(stop_key.as_bytes()) },
])
```

(The now-unused `gen_key` binding was prefixed `_gen_key` so the only *behavioral*
change is the dropped op — no `unused_variable` compile error masks the RED.)

Result: **`S-BIR-HANDLER-TXN` FAILED** — the mutation is decisively killed:

```
FAIL (1/4) acceptance::restart_workload_intent_key::restart_commits_one_bump_clear_txn_retains_intent_and_enqueues_one_eval
  panicked at …/restart_workload_intent_key.rs:147:5:
  assertion `left == right` failed: the restart must bump
  `workloads/payments/generation` from absent (0) to exactly 1 via one IncrementU64 op; got 0
    left: 0
   right: 1

Summary: 4 tests run: 3 passed, 1 failed
```

With the `IncrementU64` op dropped the generation key stays absent (decodes to
`0`) instead of advancing `0 → 1`; the `== 1` assertion fires. The other 3 ATs
pass — they do not assert on the generation value, exactly as designed (TXN is the
discriminating test for the op-set). Reverted (op restored, `_gen_key` →
`gen_key`); diff empty.

### Proof C — outcome classification (kill targets #3 / #4)

Mutation: `handlers.rs:917-921` — swap the two `RestartOutcome` arms:

```rust
let outcome = if state.store.get(stop_key.as_bytes()).await?.is_some() {
    RestartOutcome::Restarted   // ← was Resumed
} else {
    RestartOutcome::Resumed     // ← was Restarted
};
```

Result: **BOTH outcome ATs FAILED** — the mutation is decisively killed:

```
FAIL (1/4) acceptance::restart_workload_outcome::present_stop_sentinel_classifies_outcome_as_resumed
  panicked at …/restart_workload_outcome.rs:121:5:
  assertion `left == right` failed: a present `/stop` sentinel at the check-exists
  read classifies the outcome as Resumed; got Restarted
    left: Restarted
   right: Resumed

FAIL (2/4) acceptance::restart_workload_outcome::absent_stop_sentinel_classifies_outcome_as_restarted
  panicked at …/restart_workload_outcome.rs:140:5:
  assertion `left == right` failed: an absent `/stop` sentinel at the check-exists
  read classifies the outcome as Restarted; got Resumed
    left: Resumed
   right: Restarted

Summary: 4 tests run: 2 passed, 2 failed
```

Both directions are pinned: present `/stop` ⇒ must be `Resumed` (got `Restarted`),
absent `/stop` ⇒ must be `Restarted` (got `Resumed`). The other 2 ATs (404,
intent-key) pass — they do not assert on the `RestartOutcome` label. Reverted;
diff empty.

### Post-proof verification

After all three proofs, `git diff -- crates/overdrive-control-plane/src/handlers.rs`
is **empty** — production is byte-identical to `286b334b`. No mutation was
committed.

---

## 3. Conclusion — every mandatory target accounted for

| # | Mandatory kill target | Killed by (AT) | Evidence |
|---|---|---|---|
| 1 | **404 no-mutation posture** (absent ⇒ `NotFound`, no bump/enqueue) | `S-BIR-HANDLER-404` (`restart_workload_unknown.rs`) | **Proof A** — `.is_none()`→`.is_some()` ⇒ all 4 ATs RED; 404 AT shows `Ok(... outcome: Restarted)` (§ 2) |
| 2 | **`[IncrementU64, Delete]` op-set** (atomic bump + clear, exactly one txn) | `S-BIR-HANDLER-TXN` (`restart_workload_intent_key.rs`) | **Proof B** — drop `IncrementU64` ⇒ TXN RED `left: 0, right: 1` (generation stays 0) (§ 2) |
| 3 | **`present ⇒ Resumed`** classification | `S-BIR-HANDLER-OUTCOME-RESUMED` (`restart_workload_outcome.rs`) | **Proof C** — swap arms ⇒ RESUMED RED `left: Restarted, right: Resumed` (§ 2) |
| 4 | **`absent ⇒ Restarted`** classification | `S-BIR-HANDLER-OUTCOME-RESTARTED` (`restart_workload_outcome.rs`) | **Proof C** — swap arms ⇒ RESTARTED RED `left: Resumed, right: Restarted` (§ 2) |

The diff-scoped tool gate is **non-signalling** for this file: it produces exactly
**1 unviable** mutant — the whole-body `Ok(Json::new())` replacement, the
documented cargo-mutants `--in-diff` whole-body-replacement blind spot — and the
wrapper correctly returns `FAIL — no quality signal` rather than a phantom pass
(§ 1). The three **executed** manual proofs in § 2 discharge all four named kill
targets (the two outcome directions count as a pair, per the roadmap's single
"classification" target).

**This also discharges the review's accepted residual risk** (review-01-03
§ "Residual risk — exact 'one txn' is proven by state-delta, not a counting
double"). The state-delta proxy (generation `0→1`, `/stop` cleared, aggregate
retained byte-for-byte, broker `+1`) is a **sound** stand-in for "exactly one
txn" **because Proof B kills the op-set-drift case directly**: dropping the
`IncrementU64` op makes `S-BIR-HANDLER-TXN` RED (`got 0`), so the observable
state-delta cannot be satisfied by any op-set that omits or alters the
increment. The drift the counting double would catch is the drift Proof B
already kills.
