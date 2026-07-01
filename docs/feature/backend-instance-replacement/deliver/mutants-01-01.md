# Mutation evidence — step 01-01 (`TxnOp::IncrementU64`)

**Step**: `01-01` — `TxnOp::IncrementU64` atomic monotonic bump-and-clear store primitive
**Surface under gate**: `crates/overdrive-store-local/src/redb_backend.rs`
**Roadmap gate**: kill-rate ≥ 80% (`roadmap.json:22`); KILL TARGET = an `IncrementU64`
increment-direction / stale-read mutation killed by `S-BIR-TXN-02`.
**Code under test SHA**: `8ce71991` (the `txn` arm, unchanged) + `b87d49d0` (the
acceptance suite). Evidence captured at HEAD `b87d49d0`.
**Closes**: review-01-01 BLOCKER-1 ("mandatory mutation gate has no recorded evidence").

---

## TL;DR

- Whole-file gate: **11 mutants, 11 caught, 0 missed, kill-rate 100 %** — PASS, well
  above the ≥ 80 % bar.
- cargo-mutants 27.1.0 generates **zero mutants on the `txn` / `IncrementU64` arm**
  (the load-bearing surface) — a structural limitation, not a skip. Documented below.
- The `IncrementU64` increment-direction behavior is therefore proven by an **executed
  manual mutation proof** (`+1 → +0` ⇒ 5 of 6 acceptance tests fail), recorded below.
- The `--diff origin/main` scope the roadmap names is **vacuous** for this change
  (the new lines carry no mutable-operator site); the whole-file scope is the
  meaningful run. Both facts are explained below.

---

## 1. The recorded whole-file gate result

`--diff origin/main --file redb_backend.rs` reports `total_mutants=0` — a **vacuous
pass** (`.claude/rules/testing.md` § "Empty filter intersection is a vacuous pass").
Root cause: this step's changed lines in `redb_backend.rs` are the `IncrementU64`
match arm (method calls + a `match`), the `Delete` arm's `.then()`, and the
emit-payload refactor — **none of which is a cargo-mutants mutable-operator site**, and
the `txn` fn signature predates `origin/main` so its body-replacement mutant is not in
the diff. The correct meaningful scope is therefore whole-file
(`--package … --file …`), per the documented `--diff --file` trap.

Command (executed via Lima — `--features integration-tests` requires it):

```
cargo xtask lima run -- cargo xtask mutants --workspace --features integration-tests \
  --package overdrive-store-local \
  --file crates/overdrive-store-local/src/redb_backend.rs
```

Result (first-hand, captured at HEAD `b87d49d0`):

```
Found 11 mutants to test
ok      Unmutated baseline in 5s build + 4s test
11 mutants tested in 30s: 11 caught
mutants: mode=workspace total=11 caught=11 missed=0 timeout=0 unviable=0 kill_rate=100.0%
mutants: baseline=100.0% drift=+0.0pp
mutants: PASS
```

The 11 caught mutants, verbatim from cargo-mutants' `caught.txt`:

```
redb_backend.rs:146:20  delete ! in LocalIntentStore::open
redb_backend.rs:149:53  replace || with && in LocalIntentStore::open
redb_backend.rs:169:9   replace LocalIntentStore::emit with ()
redb_backend.rs:176:9   replace <…>::get -> Result<Option<Bytes>, …> with Ok(None)
redb_backend.rs:192:9   replace <…>::put -> Result<(), …> with Ok(())
redb_backend.rs:272:9   replace <…>::delete -> Result<(), …> with Ok(())
redb_backend.rs:427:9   replace <…>::scan_prefix -> Result<Vec<(Bytes,Bytes)>, …> with Ok(vec![])
redb_backend.rs:439:20  delete ! in <…>::scan_prefix
redb_backend.rs:514:9   replace <…>::bootstrap_from -> Result<(), …> with Ok(())
redb_backend.rs:543:24  delete ! in <…>::bootstrap_from
redb_backend.rs:546:57  replace || with && in <…>::bootstrap_from
```

## 2. The load-bearing arm receives no tool-generated mutant (honest finding)

Every one of the 11 mutants lands on `open / emit / get / put / delete / scan_prefix /
bootstrap_from`. **None lands in the `txn` method (lines 302-389), and none on the
`IncrementU64` arm (lines 339-371) — including the `let next = current.saturating_add(1)`
at line 362.** This was confirmed against cargo-mutants' own `caught.txt`.

cargo-mutants 27.1.0 does not synthesize a mutant here for two structural reasons:

1. `LocalIntentStore::txn`'s body is a `tokio::task::spawn_blocking(move || { … })`
   closure — it gets **no `FnValue` body-replacement mutant** (neither the outer
   `async fn` nor the closure body).
2. `current.saturating_add(1)` is a **method call with a literal argument** —
   cargo-mutants does not mutate the integer literal `1`, and does not swap
   `saturating_add`→`saturating_sub`. So the roadmap's literal "`+1 → +0`" KILL TARGET
   is **not a mutant the tool can produce**.

This is a tool limitation, not an `exclude_re` skip (`.cargo/mutants.toml` carries no
exclusion for this file). Per CLAUDE.md "report DOESN'T-WORK honestly," no caught
`txn`/`+1→+0` mutant is fabricated. (Recorded in project memory
`reference_cargo_mutants_blind_to_spawn_blocking_and_saturating_add`.)

## 3. Manual mutation proof of the increment-direction behavior (the KILL TARGET)

Because the tool cannot generate it, the `+1 → +0` mutation was applied **by hand** to
the source, the acceptance suite was run under Lima, and the source was reverted. This
is the reviewer-sanctioned "supported manual mutation proof."

Mutation: `redb_backend.rs:362` `current.saturating_add(1)` → `current.saturating_add(0)`.

Result (`cargo xtask lima run -- cargo nextest run -p overdrive-store-local --features
integration-tests -E 'test(txn_increment_u64)'`): **5 of 6 acceptance tests FAILED** —
the mutation is decisively killed:

| Test (scenario) | Outcome under `+0` | Assertion that fired |
|---|---|---|
| `single_txn_bumps_…` (S-BIR-TXN-01) | **FAIL** | `generation == 1` → got `0` |
| `concurrent_restart_txns_…` (S-BIR-TXN-02) | **FAIL** | post-commit reads ∈ `1..=N` → got all `0` |
| `absent_keys_bump_…` (S-BIR-TXN-03) | **FAIL** | `generation == 1` → got `0` |
| `corrupt_short_row_…` (S-BIR-TXN-04) | **FAIL** | `generation == 1` → got `0` |
| `sequential_bumps_…` (S-BIR-TXN-06) | **FAIL** | read after bump 1 `== 1` → got `0` |
| `generation_at_u64_max_…` (S-BIR-TXN-05) | pass (correctly) | `MAX + 0 == MAX` — the saturation test is invariant to `+0` by construction; the other five kill it |

Source reverted immediately after; `git diff -- crates/overdrive-store-local/src/redb_backend.rs`
is empty. The mutation was **never committed**.

This satisfies the roadmap's intent: the `IncrementU64` increment-direction mutation IS
killed by `S-BIR-TXN-02` (among others). A stale-snapshot read (returning a value below
the live row) would land the concurrent final value below `N` and is killed by the same
`S-BIR-TXN-02` final-`== N` + post-commit-`1..=N` assertions.

## 4. Conclusion

The mandatory mutation gate is now evidenced:

- The whole-file gate is **non-vacuous and 100 %** (11/11) — the `--diff` scope's
  `total_mutants=0` is a documented vacuous pass for this change shape, not a gap.
- The load-bearing `IncrementU64` arm — which the tool cannot reach — is covered by the
  exact-value acceptance assertions (`==1`, `==N`, strict `1..M`, `==u64::MAX`,
  post-commit `1..=N`), **proven** to kill the increment-direction mutation by the
  executed manual proof in § 3.
