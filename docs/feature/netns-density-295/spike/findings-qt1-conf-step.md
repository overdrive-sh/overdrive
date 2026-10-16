# Spike QT1: `conf_step` + `Config::nondet` for quint-connect `#[quint_test]` (D-UP-1)

**Verdict: WORKS.** With a write-only sum-typed `conf_step` state variable and `Config { nondet: &["<main>::<module>::conf_step"], .. }`, quint-connect 0.1.2 replays `#[quint_test]` named runs and `#[quint_run]` random traces against the owner. The per-step state comparison catches an owner that was broken on purpose. Without `conf_step`, the named run fails with ``Missing `mbt::actionTaken` variable in the trace``, as D-UP-1 predicts.

- **Probe:** `spike-scratch/netns-density-295-qt1/`, a standalone workspace.
  - Dependencies: `quint-connect = "=0.1.2"`, `itf = "0.4"`, `anyhow = "1"` (the `switch!` macro needs it), with the lockfile copied from root.
  - Spec in `spec/bank.qnt`, the owner and drivers in `src/lib.rs`, captures in `evidence/`.
- **Environment** (`evidence/env.txt`): Lima `overdrive`, run as root via `cargo xtask lima run --`.
  - `uname -r` = `7.0.0-34-generic`
  - `quint --version` = `0.32.0`
  - `cargo-nextest 0.9.148`
- **Command:** `cargo xtask lima run -- bash -lc 'cd spike-scratch/netns-density-295-qt1 && cargo nextest run'` (no `--no-run`).

## Setup

**Spec `bank`**, parameterised by `const REC: bool`:
- Variables: `a`, `b`, `conf_step`.
- Actions: `Init`, `Deposit(acct, amt)`, `Withdraw(acct, amt)`, `Transfer(amt)`.
- `init`/`step` pick the account and amount nondeterministically.
- Named runs `DepositThenWithdraw` and `DepositTransferDeposit` call the parameterised actions with literal arguments.
- `ConfStep` lives in its own module (`bank_conf`), because constructors named like the actions would clash with them.
- The actions build their variant with the builtin `variant("Deposit", { acct: acct, amt: amt })`, passed through `def rec(v) = if (REC) v else variant("Init", ())`.

**Mains:**

| Main | Import | Role |
|---|---|---|
| `bank_ok` | `import bank(REC = true).*` | Conformance instance |
| `bank_mc` | `import bank(REC = false).*` | Non-recording model-checking instance |
| `bank_as` | `import bank(REC = true) as inst` | Qualified instance |

**Owner:** a Rust `Bank { a, b }` with `deposit`, `withdraw` and `transfer`, projected through a `balances()` accessor.
- Every driver shares one `switch!` body: `Init`, `init`, `Deposit(acct: String, amt: i64)`, `Withdraw(..)`, `Transfer(amt: i64)`.
- The drivers differ only in `Config::nondet`.

## Observed ITF variable names (`quint test --out-itf`, no `--mbt`)

`evidence/bank_ok.itf.json`:

```
['bank_ok::bank::a', 'bank_ok::bank::b', 'bank_ok::bank::conf_step']
{'#meta': {'index': 1}, 'bank_ok::bank::a': {'#bigint': '3'}, 'bank_ok::bank::b': {'#bigint': '0'}, 'bank_ok::bank::conf_step': {'tag': 'Deposit', 'value': {'acct': 'a', 'amt': {'#bigint': '3'}}}}
```

- `evidence/bank_as.itf.json`: `['bank_as::inst::a', 'bank_as::inst::b', 'bank_as::inst::conf_step']`.
- `evidence/bank_mc.itf.json`: `conf_step` is `{'tag': 'Init', 'value': {'#tup': []}}` in every state.
- No `quint test` trace contains `mbt::actionTaken` or `mbt::nondetPicks`.

## RED: default `Config` (mbt variables) on a named run

```
== Running model based tests for red_mbt_named_run
   Generating 100 traces using `0x8404281` as random seed ...
   Replaying traces ...
   [FAIL] red_mbt_named_run
thread 'conformance::red_mbt_named_run' panicked at src/lib.rs:133:5:
Missing `mbt::actionTaken` variable in the trace
```

## GREEN: `nondet: &["bank_ok::bank::conf_step"]`

```
Starting 5 tests across 1 binary (4 tests skipped)
    PASS [   0.302s] (1/5) qt1-conf-step conformance::green_named_as_instance
    PASS [   0.303s] (2/5) qt1-conf-step conformance::green_named_deposit_transfer_deposit
    PASS [   0.305s] (3/5) qt1-conf-step conformance::green_named_deposit_then_withdraw
    PASS [   0.371s] (4/5) qt1-conf-step conformance::control_run_mbt_default_config
    PASS [   0.374s] (5/5) qt1-conf-step conformance::green_run_random
Summary [   0.374s] 5 tests run: 5 passed, 4 skipped
```

- **`green_run_random`** is `#[quint_run(main = "bank_ok", max_samples = 50, max_steps = 15)]` with the same `conf_step` config.
  - `quint run --mbt` traces carry both the `mbt::*` variables and `conf_step`.
  - quint-connect reads `conf_step` and drops the `mbt::*` variables (`src/driver/step.rs`, `extract_from_sum_type`).
- **`control_run_mbt_default_config`** uses the default config with `--mbt`. It also passes, but only after adding an `init` arm: under `--mbt`, the initial state's `actionTaken` is `init` (the wrapper), not `Init`.
- **Full suite** (`evidence/full-suite.log`): 9 run, 5 passed, 4 failed. The four failures are the RED control and the three negative probes below.

## Break the owner: the comparison is real

The `broken_owner` feature makes `Withdraw` subtract `amt - 1`. All four GREEN tests fail (`evidence/broken-owner.log`):

```
[Step 2]
Action taken: Withdraw
Nondet picks:
+ acct: "a"
+ amt: 2
Next state:
+ bank_ok::bank::a: 1
+ bank_ok::bank::b: 0
+ bank_ok::bank::conf_step: Withdraw({ acct: "a", amt: 2 })

Specification and implementation states diverge
--- specification
+++ implementation
 BankState {
-    a: 1,
+    a: 2,
     b: 0,
 }
[FAIL] green_named_deposit_then_withdraw
```

Where each of the other tests diverged:
- `green_named_deposit_transfer_deposit`: at step 4 (`b: 2` vs `b: 3`).
- `green_named_as_instance`: at step 2.
- `green_run_random`: at the first `Withdraw` of a random trace (step 4, `a: 0` vs `a: 1`).

Summary: 4 run, 0 passed, 4 failed.

## `Config::nondet` path format

`Config::nondet` is a `Path` (`&[&str]`) of record keys, not a `::`-separated string. Its first element is the ITF variable name, and `quint` writes that name fully qualified, `::` included, as one key.

| Config | Result |
|---|---|
| `&["bank_ok::bank::conf_step"]` (`.*` import) | PASS |
| `&["bank_as::inst::conf_step"]` (`as inst` import) | PASS |
| `&["conf_step"]` | FAIL: ``Can not find a Record at "conf_step" in path: ["conf_step"]`` / ``Current state: { bank_ok::bank::a: 0, bank_ok::bank::b: 0, bank_ok::bank::conf_step: Init }`` |
| `&["bank_ok", "bank", "conf_step"]` | FAIL: ``Can not find a Record at "bank_ok" in path: ["bank_ok", "bank", "conf_step"]`` |

The exact form is `nondet: &["<main>::<module>::conf_step"]`, a single element.
- With `import M(..).*`, the middle part is the declaring module's name (`bank`), not the main.
- With `import M(..) as X`, the middle part is the alias (`X`).

## Single-instance recording

Recording `conf_step` only on `bank_ok` works:
- `bank_ok` named runs and random runs pass.
- `bank_mc` (`REC = false`) typechecks, and its named run passes in `quint test` itself.
- `bank_mc`'s ITF keeps `conf_step = Init` in every state.

A conformance driver pointed at the non-recording main fails, as it must (`evidence/non-recording-main.log`). Every step decodes as `Init`, so the driver re-initialises, and the comparison catches it at step 1:

```
[Step 1]
Action taken: Init
Nondet picks: <none>
Next state:
+ bank_mc::bank::a: 3
+ bank_mc::bank::b: 0
+ bank_mc::bank::conf_step: Init

Specification and implementation states diverge
-    a: 3,
+    a: 0,
```

The failure is a loud state divergence, not a silent pass. A driver must name the recording main in its `#[quint_test(main = ..)]` / `#[quint_run(main = ..)]` attributes and in `Config::nondet`.

## Caveats

1. **State variables are qualified too.** With `Config::state = &[]`, the projected record's keys are `bank_ok::bank::a`. A plain `#[derive(Deserialize)]` state with fields `a` and `b` fails with ``Failed to deserialize specification's state.`` (seen in the first run).
   - The probe overrides the doc-hidden `State::from_spec` and matches variables by a `::<name>` suffix.
   - `Config::state` is no way around this: it is also a list of record keys and cannot select several qualified top-level variables.
   - So drivers need a projection that matches the qualified names.
2. **The middle part of the path depends on the import form:** `.*` gives the declaring module, `as X` gives the alias. D-UP-1's `<main>::<module>::conf_step` is correct for the `.*` form the repo's mains use.
3. **The init name differs between modes for drivers that rely on `mbt::*`:** `--mbt` reports the initial state as `init`. With `conf_step`, both modes report the recorded tag (`Init`), so one `switch!` serves both.
4. **Wasted repeats:** `#[quint_test]` on a deterministic named run regenerates the same trace `max_samples` times (default 100). `max_samples = 1` would do.
5. **Encoding details:** unit variants are encoded as `{'#tup': []}`, which quint-connect accepts as "no nondet picks". Parameter records must be keyed by the exact names the `switch!` arms bind (`acct`, `amt`).
6. **Not measured:** the TLC/Apalache state-space cost of `conf_step` on the non-recording instance; D-UP-1 cites `owner-flows-safety-tlc` for that. In this spike `REC = false` pins `conf_step` to a constant, so it adds one value.

## Gate recommendation

PROMOTE the D-UP-1 contract as written, with two additions:
- Pin the path as the single element `"<main>::<module>::conf_step"`.
- Add a note that state projections must match qualified variable names.
