# Testing Guidelines

Overdrive tests in four tiers. Each tier catches a class of bug the others
cannot. None of them substitutes for any other.

```
Tier 1  DST in-process            turmoil + Sim* traits        (§21)
Tier 2  BPF unit tests            BPF_PROG_TEST_RUN            (§22)
Tier 3  Real-kernel integration   QEMU + kernel matrix         (§22)
Tier 4  Verifier + perf gates     veristat, xdp-bench, PREVAIL (§22)
```

---

## Testing

**No `.feature` files anywhere.** All acceptance and integration tests are
written directly in Rust using `#[test]` / `#[tokio::test]` functions.
Gherkin-style scenarios may appear as GIVEN/WHEN/THEN blocks in
`docs/feature/{id}/distill/test-scenarios.md` for specification purposes
only — they are never parsed or executed. The crafter translates those
scenarios into Rust integration tests in
`crates/{crate}/tests/acceptance/*.rs` (or `tests/*.rs`). Do NOT
introduce cucumber-rs, pytest-bdd, conftest.py, or any `.feature` file
consumer.

---

## Integration vs unit gating

**Tests that touch real infrastructure MUST be gated behind an
`integration-tests` feature on their owning crate.** "Real
infrastructure" means anything outside the in-process pure-Rust
fixture envelope:

- Real filesystem I/O — opening real `redb` / `libSQL` / `sqlite`
  files, writing to `tempfile::TempDir`, mmap'ing on-disk artifacts.
- Real network — binding sockets, contacting localhost services,
  spawning a real Corrosion peer, real Raft transport.
- Real subprocesses — `Command::spawn`, `cargo run`, real Cloud
  Hypervisor, real `wasmtime` outside the in-process API.
- Real consensus / gossip — `RaftStore` with multiple peers, real
  CR-SQLite LWW with multiple sites.
- Heavy property-based suites — proptest invocations whose default
  case count drives the per-test wall-clock past the 60s nextest
  slow-test budget (e.g. 1024 cases × real redb roundtrip).

The default unit/proptest suite covers in-process logic exclusively
— `Sim*` traits, `LocalStore` against a tiny dataset, hand-picked
proptest cases, trybuild compile-fail. Anything heavier moves out of
the default lane.

### Workspace convention — every member declares the feature

**Every workspace member MUST declare `integration-tests = []` in its
`[features]` block, even crates that have no integration tests of
their own.** For crates without integration tests the declaration is a
deliberate no-op; for crates with them it gates the slow lane (see
*Mechanics* below).

Why: `cargo xtask mutants --features integration-tests` (the canonical
mutation invocation; CI passes it on every PR) propagates the bare
feature flag to per-mutant cargo invocations. cargo-mutants v27.0.0
scopes those invocations to `--package <owning-crate>`. Cargo's
feature resolution requires the scoped package to declare the feature
— a non-declaring package in the mutation surface produces
`error: the package 'X' does not contain this feature: integration-tests`
and marks every mutant under it unviable, collapsing the kill-rate
signal to zero. The convention makes the bare flag universally valid.

Enforcement is automated: `xtask::mutants::tests::every_workspace_
member_declares_integration_tests_feature` walks the workspace
`members` list and fails the PR if any member is missing the
declaration. A new crate added without it cannot land.

The narrative behind the convention — and the original failure mode
that motivated it — lives in PR #132's RCA (April 2026).

### Mechanics

Declaration shape, identical for every member:

```toml
[features]
# Workspace-wide convention. Every workspace member declares this
# feature so `cargo {check,test,mutants} --features integration-tests`
# resolves uniformly under per-package scoping. For crates with
# integration tests, this is the gate; for crates without, the
# declaration is a deliberate no-op. See § "Integration vs unit
# gating" above.
integration-tests = []
```

**Layout — integration tests live under `tests/integration/`.** All
integration-shaped tests for a crate go in
`crates/{crate}/tests/integration/<scenario>.rs`, wired through a
single `crates/{crate}/tests/integration.rs` entrypoint:

```
crates/<crate>/tests/
  integration.rs              # entrypoint, gates the whole binary
  integration/
    <scenario_a>.rs           # one file per scenario
    <scenario_b>.rs
```

`tests/integration.rs` carries the feature gate at the top of the
file; the per-scenario modules under `tests/integration/` inherit it
and do not repeat the cfg attribute. **Submodules must be declared
inside an inline `mod integration { … }` block** — Cargo treats each
`tests/*.rs` file as a crate root, so a bare `mod foo;` resolves to
`tests/foo.rs`, not `tests/integration/foo.rs`. The inline wrapper
shifts the lookup base into the subdirectory. This is the same trick
`tests/acceptance.rs` uses today.

```rust
// tests/integration.rs
#![cfg(feature = "integration-tests")]

mod integration {
    mod <scenario_a>;
    mod <scenario_b>;
}
```

```rust
// tests/integration/<scenario_a>.rs
// (no cfg attribute — the entrypoint gates the whole binary)
use proptest::prelude::*;
// ... test code ...
```

The default `cargo nextest run` does not compile the entrypoint, so
the per-scenario files are never reachable. CI runs a dedicated
`integration` job that enables the feature explicitly via
`--features <crate>/integration-tests`. Filter the slow lane with
`-E 'binary(integration)'` so the unit suite is not re-run.

**Why a dedicated subdirectory.** Plain `tests/<scenario>.rs` would
work mechanically, but two adjacent files at `tests/` top level —
one fast-and-default, one slow-and-gated — read the same to a
reviewer and invite accidental ungating. The
`tests/integration/<scenario>.rs` layout makes the lane visible at
file-system level, mirrors the `tests/acceptance/<scenario>.rs`
convention from ADR-0005, and gives the CI filter a single
`binary(integration)` selector that does not need updating per
scenario.

### What stays in the default lane

A test belongs in the default lane only if every one of these holds:

- Runs in well under 60 seconds wall-clock on a developer laptop.
- Performs no real I/O outside of in-memory sim traits.
- Does not depend on subprocess spawning, real network sockets, or
  real consensus / gossip.
- Default proptest case count completes within the wall-clock budget
  above (do NOT lower `PROPTEST_CASES` to dodge the budget — gate the
  test instead).

If a test outgrows the default lane (real infra creeps in, the
proptest grows, etc.), move it under `integration-tests` immediately.
Do not let a slow test sit in the default lane "until it gets fixed."

### What this is NOT

- Not a substitute for any of the four tiers above. Tier 1 (DST) is
  still pure-Rust under sim traits and stays in the default lane.
  Tier 2/3/4 still ship via `cargo xtask {bpf-unit,integration-test
  vm,verifier-regress,xdp-perf}` — those wrappers own their own
  scheduling.
- Not "integration test" in the colloquial Rust sense (`tests/*.rs`).
  Many `tests/*.rs` files are pure unit-shaped and stay in the
  default lane. The gate is about *real infra*, not about file
  location.

---

## Running tests — foreground, always

> **The test runner is `cargo nextest run`. Not `cargo test`.**
>
> `cargo test` is **blocked** by a pre-tool hook
> (`.claude/hooks/block-cargo-test.ts`) outside the one legitimate case:
> `cargo test --doc ...` (nextest cannot execute doctests). Every other
> shape — `cargo test`, `cargo test -p foo`, `cargo test --workspace`,
> `cargo test -- <filter>` — is rejected at the tool-call boundary.
>
> **Rewrite your command before submitting it:**
>
> | ❌ don't | ✅ do |
> |---|---|
> | `cargo test` | `cargo nextest run` |
> | `cargo test -p CRATE` | `cargo nextest run -p CRATE` |
> | `cargo test -p CRATE --lib` | `cargo nextest run -p CRATE --lib` |
> | `cargo test -p CRATE --test acceptance` | `cargo nextest run -p CRATE --test acceptance` |
> | `cargo test --workspace --locked` | `cargo nextest run --workspace --locked` |
> | `cargo test -- <filter>` | `cargo nextest run -E 'test(<filter>)'` |
> | `cargo test -- --nocapture` | `cargo nextest run --no-capture` |
> | `cargo test --features X` | `cargo nextest run --features X` |
>
> The **only** allowed `cargo test` is `cargo test --doc ...` for rustdoc
> examples. Nothing else. If you think you need `cargo test` elsewhere,
> you are wrong — reach for `cargo nextest run` instead.

> **On macOS, every `cargo nextest run` must go through Lima.**
>
> `cargo nextest run` is **blocked on macOS** by a pre-tool hook
> (`.claude/hooks/block-nextest-on-macos.ts`) unless it is already
> wrapped in `cargo xtask lima run --` or uses `--no-run`.
>
> **Rewrite your command before submitting it on macOS:**
>
> | ❌ don't | ✅ do |
> |---|---|
> | `cargo nextest run` | `cargo xtask lima run -- cargo nextest run` |
> | `cargo nextest run -p CRATE` | `cargo xtask lima run -- cargo nextest run -p CRATE` |
> | `cargo nextest run -E 'test(X)'` | `cargo xtask lima run -- cargo nextest run -E 'test(X)'` |
> | `cargo nextest run --workspace` | `cargo xtask lima run -- cargo nextest run --workspace` |
>
> **Allowed on macOS without Lima:**
> - `cargo nextest run ... --no-run` — compile-check only; no Linux surface involved.
> - `cargo xtask lima run -- cargo nextest run ...` — already routed.
>
> See § "Running tests on macOS — Lima VM" below for the rationale.

**Run test commands directly. Do not background them.**
`cargo nextest run`, `cargo test --doc`, `cargo xtask dst`,
`cargo xtask bpf-unit`, `cargo xtask integration-test`, and every other
test invocation goes through the `Bash` tool with
`run_in_background: false` (the default). Wait for the command to finish;
read the full output in the tool result.

**Runner: `cargo-nextest`.** The project-wide runner is
[`cargo-nextest`](https://nexte.st). `cargo test` is reserved for
*doctests only* — nextest does not execute them. Every nextest
invocation in CI is paired with a `cargo test --doc` counterpart; in
lefthook, doctests run scoped per-crate at *pre-commit* time (tight
feedback for rustdoc examples), while the nextest suite runs at
pre-push. Profile config lives in `.config/nextest.toml`.

**Quiet output by default.** The `default` profile sets
`status-level = "fail"` and `final-status-level = "fail"` — clean runs
print a one-line summary, failing runs show the failures directly
rather than burying them under dozens of PASS lines. This removes the
usual reason to reach for `| tail -N`, which eats the non-zero exit
code (bash pipelines do not enable `pipefail` per tool call). When
`--no-fail-fast` multi-failure inspection still warrants piping (for
example `cargo nextest run ... --no-fail-fast 2>&1 | grep -E
'^(\s+(PASS|FAIL|SKIP)|Summary|test result)' | tail -30`), the
PostToolUse hook `.claude/hooks/flag-test-failure.ts` scans the
captured output for FAIL markers and nextest summaries reporting
N failed, so exit-code loss cannot silently mask a failure.

- **Do NOT** set `run_in_background: true` on a test command and then
  poll with `tail`, `cat`, `wait`, or sleep loops against the output
  file. Each poll burns a turn, the harness blocks long `sleep`s, and
  you lose the structured output view that the direct tool result
  gives you.
- **Do NOT** redirect test output to a temp file and `tail` it. The
  tool already captures stdout+stderr, and the `default` nextest
  profile only surfaces failures anyway. Piping to `| tail -N`
  in-command is acceptable when you specifically need the tail of a
  `--no-fail-fast` multi-failure inspection — skip it otherwise, since
  the pipe masks the exit code. The PostToolUse failure scanner is the
  backstop when piping is unavoidable, not a licence to pipe by
  default.
- **Prefer running only the affected tests.** Default to
  `cargo nextest run -p <crate>` for the crate you changed, or
  `cargo nextest run -p <crate> -E 'test(<filter>)'` for a specific
  test. A whole-workspace run is the exception — reserve it for the
  final pre-commit check or when a change crosses crate boundaries.
- **Doctests are a separate step.** `cargo test --doc -p <crate>` when
  you touch a rustdoc example; `cargo test --doc --workspace` before
  push. Never rely on nextest to catch a broken doctest.
- **Long-running suites are still foreground.** When a whole-workspace
  run is genuinely warranted, set a `timeout` up to 600000ms (10 min)
  and let it run. Minutes of waiting is cheaper than poll cycles that
  each re-read context.
- **The only exception** is a genuinely concurrent workflow — e.g. you
  need to run the test while also editing unrelated files. Even then,
  prefer to let the test finish first; backgrounding is rarely the
  right tradeoff for a test run.

### Mutation testing is the exception

`cargo mutants` and `cargo xtask mutants` — and only these — are
exempt from the foreground-only rule. A mutation run does a full
build + nextest rerun per mutant; even diff-scoped
(`--in-diff origin/main`) the wall clock exceeds the 10 min Bash tool
cap on this workspace. Running it foreground means it gets SIGKILLed
at the cap, which is both misleading (the run wasn't actually
failing) and destructive (`pkill -f "cargo mutants"` then leaves
mutated source on disk — see the Post-Mutation Safety step in
`nw-mutation-test`).

Rules for mutation specifically:

- **Invoke with `run_in_background: true`.** Capture stdout+stderr to
  a file; the command returns immediately with a shell ID.
- **Wait for exit, don't poll output.** Use the shell ID's completion
  signal. Do not `tail -f` the log file in a loop — each read burns a
  turn and the real signal is "the process exited," not "the log
  looks done."
- **No wall-clock kill.** If the run is taking longer than expected,
  that's information — either the diff scope is wrong, the
  `.cargo/mutants.toml` skip list is incomplete, or the test suite is
  slower than budgeted. Investigate; do not `pkill`.
- **Always run the post-mutation safety step** (`git checkout --
  <src>`) whether the run succeeded, failed, was cancelled by the
  user, or errored. Mutated source on disk is the failure mode that
  makes this exception load-bearing.

This exception is narrow. Nextest, `cargo test --doc`, `cargo xtask
dst`, `cargo xtask bpf-unit`, `cargo xtask integration-test vm`,
`cargo xtask verifier-regress`, and `cargo xtask xdp-perf` all stay
foreground.

---

## RED scaffolds and intentionally-failing commits

Outside-In TDD produces intentionally-failing test scaffolds: new
`SimInvariant` variants, new arms in an exhaustive match, new trait
methods the harness calls before the implementation exists. Mark the
unimplemented branch with `panic!("Not yet implemented -- RED
scaffold")` (or `todo!("RED scaffold: ...")`). The panic IS the
specification of work not yet done.

**Downstream fallout on pre-existing tests is expected and correct.**
When a generic harness iterates every variant — DST walking every
`SimInvariant`, a property test enumerating every action, a match
covering every driver class — the new RED branch makes pre-existing
tests panic the moment they touch it. Do NOT "fix" this by replacing
the `panic!` with a neutral stub (`Ok(())`, `Verdict::Allow`, `return
vec![]`). A neutral stub turns the bar green and masks the
unfinished state — the whole point of the RED phase is that the bar
is red until the implementation lands.

When a pre-commit or pre-push hook fires on a RED scaffold:

- **Do not** swap the `panic!` for a neutral stub to satisfy the gate.
- **Do not** add `#[ignore]` to the pre-existing tests the new
  scaffold now panics. That hides a regression surface the moment the
  scaffold goes GREEN — the paired tests are precisely what will
  validate the implementation.
- **Commit with `git commit --no-verify`** and call it out explicitly
  in the commit message or user-facing summary. Intentionally-RED
  commits are one of the explicit exceptions to "never skip hooks":
  forcing green with stubs is worse than acknowledging red. Full
  pre-push lefthook and CI still catch anything that shouldn't ship;
  the pre-commit gate is here to catch accidents, not to block the
  GREEN-next-commit loop.

---

## Tests that mutate process-global state (`serial_test`)

Some tests need to mutate state that is shared across the whole
process — most commonly environment variables (`$HOME`,
`$OVERDRIVE_CONFIG_DIR`, `$XDG_DATA_HOME`), the current working
directory, or a global `OnceLock` used by production code. Nextest
runs tests in parallel threads within a single binary by default,
so two tests touching the same env var race and corrupt each other's
fixtures. The fix is serialisation, not coarser locks.

**Use [`serial_test`](https://docs.rs/serial_test) with
`#[serial(env)]`** for every test that mutates a process-global
resource. Dev-dep only, declared once in
`[workspace.dependencies]` and pulled into the consuming crate via
`serial_test.workspace = true`.

```rust
use serial_test::serial;

#[tokio::test]
#[serial(env)]
async fn honours_home_env_var_fallback() {
    let _guard = EnvGuard::scoped(&[("HOME", Some(tmp.path())),
                                    ("OVERDRIVE_CONFIG_DIR", None)]);
    // ... test body — env is exclusive for the duration of this test
}
```

### Rules

- **`#[serial(env)]`, not bare `#[serial]`.** The `(env)` key groups
  env-mutating tests together; other `#[serial(...)]` groups (and
  all un-annotated tests) continue running in parallel. Bare
  `#[serial]` serialises against *every other* `#[serial]` test in
  the process, which is coarser than needed and slower.
- **Always wrap env mutations in an explicit `unsafe { }` block.**
  Rust 2024 + workspace-wide `unsafe_op_in_unsafe_fn = deny` makes
  this mandatory — `std::env::set_var` / `remove_var` / `set_current_dir`
  are `unsafe fn` and the compiler rejects bare calls. Add a
  `// SAFETY:` comment explaining *why* it is safe (typically:
  "`#[serial(env)]` guarantees exclusive access for the duration of
  this test").
- **Save and restore via RAII, never by hand.** Assertion failures
  panic mid-test; leaked env state corrupts subsequent tests in the
  same binary. Use an `EnvGuard`-shaped helper (saves prior values
  in `new()`, restores in `Drop`) rather than manual
  save-mutate-restore, so the restore runs even when the test
  panics. One such helper per crate that needs it; don't scatter
  ad-hoc save-restore dances across test files.
- **Workspace dev-dep.** Declare `serial_test = "3"` (or current
  stable major) under the `# --- Testing ---` block of
  `[workspace.dependencies]` in the root `Cargo.toml`. Consuming
  crates add `serial_test.workspace = true` under
  `[dev-dependencies]`. Never pin a version in a leaf crate per
  `development.md` — Dependencies.
- **Every test that *reads* the mutated resource is still fair
  game.** `#[serial(env)]` only locks out *other `#[serial(env)]`
  tests*. An un-annotated test that reads `$HOME` while an
  annotated test mutates it still races. If a variable is in play,
  every test that touches it gets the annotation — not just the
  writers.

### What NOT to use instead

- **Crate-local `std::sync::Mutex` guard.** Works mechanically, but
  every new env-mutating test must remember to take the lock AND
  remember to save/restore manually — a landmine that produces
  phantom nextest failures the first time a third test lands in the
  same binary without the guard. `serial_test` makes the correct
  pattern declarative; the mutex pattern makes it discretionary.
- **Separate test binary with `threads = 1` via a nextest profile
  override.** Overkill for a concurrency-of-2 constraint; splits
  the `tests/integration/<scenario>.rs` layout without buying
  anything a `#[serial(env)]` annotation doesn't, and costs CI
  minutes on extra binary spin-up.
- **Bare `#[serial]` (no group key).** Serialises against every
  other `#[serial]` test — too coarse; slows unrelated suites.
- **`std::env::set_var` without `unsafe { }`.** Fails to compile on
  Rust 2024. Don't try to silence with `#[allow(unsafe_op_in_unsafe_fn)]`
  — the workspace-wide `deny` is load-bearing for the rest of the
  codebase.

### When it's not the answer

`serial_test` fixes *process-shared* state. It does NOT fix:

- **Shared filesystem paths.** Two tests both writing to
  `/tmp/overdrive-cache` race regardless of env locks. Use
  `tempfile::TempDir` (per-test) instead.
- **Shared port numbers.** Two tests both binding `127.0.0.1:7001`
  race on the OS kernel, not on the process. Bind `127.0.0.1:0`
  and read the assigned port.
- **Shared global state the production code caches.** A `OnceLock`
  populated on first access stays populated for the whole process;
  no test-level lock can reset it. Either inject the state via a
  constructor parameter in production code, or accept that the
  test order matters and gate behind `integration-tests`.

---

## Tier 1 — Deterministic Simulation Testing

### Nondeterminism must be injectable

Every source of nondeterminism in core logic is behind a trait. No exceptions:

| Trait | Real | Sim |
|---|---|---|
| `Clock` | `SystemClock` | `SimClock` (turmoil) |
| `Transport` | `TcpTransport` | `SimTransport` (turmoil) |
| `Entropy` | `OsEntropy` | `SeededEntropy` (StdRng) |
| `Dataplane` | `EbpfDataplane` | `SimDataplane` (HashMap) |
| `Driver` | `CloudHypervisorDriver` etc. | `SimDriver` |
| `IntentStore` | `LocalStore` / `RaftStore` | `LocalStore` |
| `ObservationStore` | `CorrosionStore` | `SimObservationStore` (in-memory LWW) |
| `Llm` | `RigLlm` | `SimLlm` (transcript replay) |

Rules:

- **Never call `Instant::now()`, `SystemTime::now()`, `Duration::from_secs` +
  `tokio::time::sleep`, `rand::random()`, `rand::thread_rng()`, or
  `std::thread::sleep` in core logic crates.** These are allowed only in
  wiring crates where the real implementations of the traits live.
- **Never spawn raw `TcpStream`, `TcpListener`, or `tokio::net::*` in core
  logic.** Go through `Transport`.
- **Never call the kernel or `aya-rs` directly from control-plane logic.**
  Go through `Dataplane`.
- A lint / grep CI gate enforces the above at the crate boundary. If the
  gate flags your code, the fix is a new method on the trait, not a
  bypass.

### What to write as DST

Every control-plane behaviour whose correctness depends on ordering, timing,
concurrency, or partition tolerance. If the behaviour is "single node,
single thread, no clock dependency," a plain `#[test]` is fine — DST is the
wrong tool.

Concrete must-haves:

- Leader election under partition, clean crash, clock skew.
- Scheduler placement under concurrent job submission.
- Certificate rotation across leader changes.
- Reconciler convergence after node rejoins.
- Corrosion gossip under peer event-loop stalls (the Fly contagion
  scenario is a *named* scenario, not a hypothetical).
- Investigation agent against seeded `SimLlm` transcripts — deviation in
  tool choice or parameter shape fails the test.

### Properties, not scenarios

Prefer invariants over scripted assertions. Three categories:

```rust
// Safety — nothing bad ever happens
assert_always!("single leader",
    cluster.nodes().filter(|n| n.is_leader()).count() <= 1);

assert_always!("intent never crosses into observation",
    corrosion.tables().all(|t| !t.contains_intent_class()));

// Liveness — good things eventually happen
assert_eventually!("job scheduled",
    submitted_jobs.iter().all(|j| j.has_allocation()));

// Convergence — reconcilers reach desired state
assert_eventually!("desired == actual",
    desired_state == actual_state);
```

Rules:

- Every built-in reconciler ships with ESR specifications (progress +
  stability) expressible as `assert_always!` / `assert_eventually!` pairs.
- A scripted scenario test without an invariant is a smell — ask what the
  scenario is actually defending.

### Seeding and reproducibility

- Every DST test takes a seed. On failure, the harness prints the seed.
- `cargo xtask dst --seed <N>` reproduces bit-for-bit.
- Flaky DST is a bug in the sim layer, never a "just rerun it." Fix or
  file.

### Store composition

Four store modes. Use the narrowest that exercises the behaviour:

- `LocalStore` + `SimObservationStore` — single-node, most tests.
- `RaftStore` + `SimObservationStore` — consensus tests only.
- `LocalStore` + real `CorrosionStore` — cross-region gossip tests (these
  are slower; reserve for the behaviour that requires real CR-SQLite LWW
  semantics).
- `RaftStore` + real `CorrosionStore` — full-stack integration, sparing.

---

## Property-based testing (proptest)

Complements Tier 1. DST catches bugs from concurrency, timing, ordering,
and partition; proptest catches bugs from inputs the author didn't think
of. Neither substitutes for the other.

### Mandatory call sites

- **Newtype roundtrip.** Every newtype's `Display` / `FromStr` / serde
  must round-trip bit-equivalent for every valid input, and every invalid
  input must be rejected by `FromStr` with a structured `ParseError`. See
  `development.md` — Newtype completeness.
- **rkyv roundtrip.** Archive → access → deserialise → equal-to-original
  for every durable type crossing the IntentStore boundary.
- **Snapshot roundtrip.** `IntentStore::export_snapshot` →
  `bootstrap_from` → `export_snapshot` is bit-identical. The
  non-destructive single → HA migration story depends on this.
- **Hash determinism.** Any content hash under `development.md`'s
  "Hashing requires deterministic serialization" rule — N permutations of
  the same logical value must produce one hash.

Reach for it elsewhere whenever a pure function's argument space exceeds
a dozen hand-picked cases. If you find yourself enumerating `#[test]`
bodies — "case 1, case 2, case 3, …" — the function wants a proptest
instead.

**Not for** concurrency, partition, or timing — that is Tier 1. Not for
kernel attachment — that is Tier 3.

### Rules

- **Seed printed on failure.** Reproduce with
  `PROPTEST_CASES=1 PROPTEST_REPLAY=<seed> cargo nextest run -E 'test(<name>)'`
  (or `cargo test --doc <name>` if the failing case is a doctest).
- **Shrink before filing.** Never file a bug against a raw failure — let
  proptest minimise the counter-example first. Unshrunk reports hide the
  actual trigger.
- **Flaky proptest is a bug, not reality.** Same discipline as flaky DST:
  fix the generator or the code under test; never "just rerun it."
- **Generators live next to the types they produce.** `SpiffeId` owns its
  `Arbitrary` impl; tests import it. Don't scatter ad-hoc generators
  across crates.
- **CI runs the default case count per PR** (`PROPTEST_CASES=1024`).
  Per-release soaks run higher. Don't lower the default to dodge a slow
  generator — fix the generator.

---

## Compile-fail testing (trybuild)

Complements Tier 1 and proptest. DST perturbs schedules; proptest perturbs
inputs; [`trybuild`](https://docs.rs/trybuild) proves **compile-time
invariants** — that invalid code fails to compile with a predictable
diagnostic. Used sparingly: only when a type-system property is
load-bearing and the failure mode is "someone calls the wrong API with
the wrong type."

### Mandatory call sites

- **Intent vs observation non-substitutability.** `&dyn IntentStore` must
  NOT be usable where `&dyn ObservationStore` is expected, and vice
  versa. Without a compile-fail test, a future refactor could blur the
  two traits and nothing would catch it — the state-layer discipline
  would silently erode. Lives under
  `crates/overdrive-core/tests/compile_fail/`.
- **Typed write surface.** `ObservationStore::write` takes
  `ObservationRow`, not `&[u8]`. Passing an intent-class value (e.g. a
  `JobSpec`) must fail to compile, not at runtime.

Reach for it elsewhere only when the type system IS the invariant. If the
check could be a `#[test]` assertion, prefer the `#[test]`.

### Rules

- **Check in the `.stderr` file** next to each fixture. The failure
  output IS the assertion — flaky diagnostics are a bug in trybuild (pin
  the version) or in your test (simplify the fixture).
- **Pin trybuild exactly** (e.g. `trybuild = "=1.0.101"`). The stderr
  format is sensitive to both rustc and trybuild itself; a floating
  version breaks the check on every upstream release. Update the pin
  deliberately, regenerate the `.stderr` files, review the diff.
- **One invariant per fixture.** A file that fails for two reasons is a
  flaky test — which reason fired is non-deterministic.
- **Not for syntax errors or incidental type mismatches.** Reserve it for
  deliberate type-system properties the project depends on.

---

## Mutation testing (cargo-mutants)

Complements Tier 1 and proptest. DST perturbs **schedules**; proptest
perturbs **inputs**; [`cargo-mutants`](https://mutants.rs) perturbs
**code**. The question each answers is distinct:

- DST: does the logic hold under every interleaving?
- proptest: does the logic hold for every input?
- mutants: do the tests actually assert on the thing that matters?

A passing suite with weak assertions is indistinguishable from a strong
one until mutants finds the blind spots. `cargo-mutants` applies small,
targeted edits — flipping a boolean, swapping `<` for `<=`, replacing a
function body with `Default`, erasing a match arm — and reruns the test
suite per mutation. Outcomes:

- **Caught** — at least one test failed. The mutation was killed.
- **Missed** — every test passed despite the change. Either a test is
  missing, or the existing tests don't assert on the changed behaviour.
- **Timeout / unviable** — investigate; these are not freebies.

### Usage

Mutation testing goes through the `cargo xtask mutants` wrapper, not
`cargo mutants` directly. The wrapper materialises the diff file
cargo-mutants expects, pins `--test-tool=nextest` to match the project
runner, writes `target/xtask/mutants-summary.json`, and implements the
kill-rate gate. Invoking `cargo mutants` directly skips all of that.

**On macOS: every mutation invocation that includes `--features
integration-tests` MUST be prefixed with `cargo xtask lima run --`** —
same Lima requirement that governs `cargo nextest run --features
integration-tests` (see § "Running tests on macOS — Lima VM" below).
The integration-tests-gated test surface is
`#[cfg(target_os = "linux")]`; on macOS those tests compile (with
`--no-run`) but the runtime surface is unreachable, so a mutation run
that supposedly uses these tests has a degraded signal that catches
almost nothing — kill rate becomes meaningless. Every example in this
section that shows a `cargo xtask mutants ... --features
integration-tests` invocation runs on macOS as `cargo xtask lima run
-- cargo xtask mutants ... --features integration-tests`. Mutation
runs without `--features integration-tests` (rare; the
"deliberately measuring kill rate without acceptance tests"
escape-hatch example below) MAY run directly on macOS — Lima is only
required when integration tests are participating. CI runs on Linux
and does not need the prefix.

Two modes, mutually exclusive (clap rejects both-or-neither):

```bash
# Per-PR, diff-scoped. Gate: kill rate ≥ 80%. This is the default
# CI invocation. `--features integration-tests` is the canonical
# feature flag — every workspace member declares it (see § "Workspace
# convention" above), so the bare flag resolves uniformly under
# cargo-mutants v27's per-package scoping. The wrapper does not
# auto-add anything; pass it explicitly.
cargo xtask mutants --diff origin/main --features integration-tests

# Nightly / full-corpus. Gate: ≥ 60% absolute floor; drift ≤ -2pp vs.
# mutants-baseline/main/kill_rate.txt is a soft-warn.
cargo xtask mutants --workspace --features integration-tests

# Override the baseline path for --workspace (rare; default is usually
# correct):
cargo xtask mutants --workspace --features integration-tests \
  --baseline mutants-baseline/main/kill_rate.txt
```

**Scope narrowing.** The wrapper exposes cargo-mutants' own `--file`,
`--package`, and `--features` as pass-throughs. These compose with
`--diff` or `--workspace`:

```bash
# Single file in a single package — the canonical TDD-inner-loop
# invocation. The wrapper adds --test-workspace=false automatically
# when --package is set (only that crate's test suite is rerun per
# mutation — large wall-clock win).
cargo xtask mutants --diff origin/main --features integration-tests \
  --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/handlers.rs

# Whole package, no diff scoping:
cargo xtask mutants --workspace --features integration-tests \
  --package overdrive-core

# Extra features on top of integration-tests:
cargo xtask mutants --diff origin/main \
  --features integration-tests,unstable-foo,bar \
  --package overdrive-control-plane

# Drop integration-tests (rare; deliberately measuring kill rate
# without acceptance tests participating):
cargo xtask mutants --diff origin/main \
  --package overdrive-control-plane

# Opt out of the --test-workspace=false default when mutations in one
# crate can only be killed by tests in another:
cargo xtask mutants --diff origin/main --features integration-tests \
  --package overdrive-control-plane \
  --test-whole-workspace
```

**Per-step vs per-PR scoping.** Running the unscoped
`cargo xtask mutants --diff origin/main --package <crate>` after every
DELIVER step takes 15+ minutes and is the wrong default for the inner
loop — by the end of a multi-step feature you have re-mutated earlier
commits' code dozens of times. Two-tier discipline:

- **Per DELIVER step (inner loop).** Pass `--file` for every file you
  touched in *this* step. Mutates only the lines you just changed; PR-wide
  gate semantics are preserved (still `--diff origin/main`), and
  `--test-workspace=false` keeps the rerun scoped to the package's own
  tests.

  ```bash
  cargo xtask mutants --diff origin/main \
    --package overdrive-store-local \
    --file crates/overdrive-store-local/src/<file-from-this-step>.rs \
    --file crates/overdrive-store-local/src/<another-file>.rs
  ```

- **Once before opening the PR (final check).** Drop the `--file` flags
  and run the full per-package diff. This is the gate CI runs; it must
  pass before merge regardless of how many times the per-step scoped
  runs passed.

  ```bash
  cargo xtask mutants --diff origin/main --package overdrive-store-local
  ```

If a step did not touch any source the mutation gate covers (docs-only,
test-only, schema migration with no logic, generated code), skip the
mutation run for that step entirely — `--file` with no logic source has
nothing to mutate. The final per-PR run still catches anything missed.

**Empty filter intersection is a vacuous pass.** When `--file` (or
`--file` × `--diff`) names paths whose diff lines do not overlap a
mutable mutation operator, cargo-mutants logs `INFO No mutants to filter`
and exits 0 without producing `outcomes.json`. The wrapper treats this
as a vacuous pass — kill rate is undefined, the gate is satisfied, and
`target/xtask/mutants-summary.json` records `total_mutants=0` with
`status="pass"`. If you expected mutants and got zero, double-check
that the file actually changed against the diff base
(`git diff <base> -- <file>`) and that the change includes mutable
operator sites (return values, comparison operators, match arms — not
just whitespace, comments, or rustdoc).

**Why `--features integration-tests` is explicit, not auto-added.**
This repo's acceptance tests live behind `#[cfg(feature =
"integration-tests")]` per §"Integration vs unit gating". Without the
feature enabled, those tests don't compile — which means cargo-mutants
runs the mutation against a build where the tests that would catch it
are absent, and kill rate is silently understated. CI passes the flag
explicitly on every mutants invocation; the wrapper does not auto-add
it. The workspace convention (every member declares
`integration-tests = []`, even no-op crates — see §"Workspace
convention" above) is what makes the bare flag safe under cargo-mutants
v27's per-package scoping. An earlier auto-add scheme tried to emit
per-package qualified `<pkg>/integration-tests` features for crates
that declared them; that scheme broke when v27 scoped per-mutant
builds to `--package <owner>` and cargo refused cross-package
qualifiers — see PR #132's RCA (April 2026).

**Flag confusion to avoid.** `cargo-mutants` itself takes
`--in-diff <FILE>` — a *file path*, not a git ref. The xtask wrapper
takes `--diff <BASE_REF>` — a git ref — and handles the diff
materialisation. Passing `--in-diff` to `cargo xtask mutants` will
fail; passing `--diff` to bare `cargo mutants` will fail. Use the
wrapper with `--diff` and neither mistake comes up.

**Invocation shape.** Mutation is the explicit exception to the
foreground-only rule above (see "Mutation testing is the exception"
at the top of "Running tests"):

- Use `run_in_background: true`. A mutation run over a real diff
  regularly exceeds the 10 min Bash tool cap; foreground execution
  will SIGKILL mid-run.
- Let it finish. The wrapper exits 0 on pass, non-zero on gate
  failure. Do NOT `pkill -f "cargo mutants"` when it seems slow —
  that leaves mutated source on disk and skips
  `target/xtask/mutants-summary.json` generation.
- After every run (pass, fail, cancel), run `git checkout -- crates/`
  to restore any mutated source the wrapper didn't clean up itself.
  This is a belt-and-braces step, not a substitute for letting the
  run finish.

**Reading the output.** The summary file at
`target/xtask/mutants-summary.json` is the structured gate record —
it contains kill rate, caught/missed/timeout/unviable counts, and the
gate verdict. Parse that, not the human-readable stdout. CI reads
the same file for the `GITHUB_STEP_SUMMARY` annotation.

**Installation.** Both `cargo-mutants` and `cargo-nextest` must be on
PATH — the wrapper checks for both up front and bails with an
install hint if either is missing.

### Mandatory targets

Every merge into `main` must meet a **kill rate ≥ 80%** on the code below
(matching the `nw-mutation-test` skill threshold). Missed mutations are
reviewed per-PR, not aggregated across releases.

- **Reconciler logic.** Every `reconcile(desired, actual, db) →
  Vec<Action>` implementation. A missed mutation here means a behaviour
  the suite does not actually defend.
- **Policy verdict compilation.** Regorus input → BPF map bytes. A
  mutation that changes the compiled verdict must fail a test, or the
  "policy enforced" claim is unverified.
- **Newtype `FromStr` and validators.** Every accept/reject branch must
  have at least one test that flips on the mutation. Case-insensitivity,
  canonical form, and structured `ParseError` variants are all mutation
  targets.
- **Hash determinism paths.** The rkyv-archive / JCS canonicalisation
  code per `development.md` — mutations that change the hash output must
  fail a test, otherwise the content-addressed ID story is hollow.
- **Scheduler bin-pack.** Placement decisions and the constraint
  evaluator. Mutations that swap `>` for `>=` in capacity checks are the
  canonical bug shape this catches.
- **`IntentStore::export_snapshot` / `bootstrap_from`.** Single → HA
  migration correctness rides on this code; a missed mutation means the
  roundtrip proptest isn't actually closing the loop.
- **Workflow `run` bodies.** Replay-equivalence depends on each `await`
  point producing the right action. A missed mutation on a `ctx` call
  means the replay harness isn't pinning the trajectory.

### Rules

- **Scoped per PR.** `cargo xtask mutants --diff origin/main` runs
  only mutations that overlap the PR diff. Full-corpus runs are nightly;
  per-PR budget is tight.
- **One-line skip with justification.** Use `// mutants: skip` above
  blocks that are genuinely untestable (panic paths, trivial getters
  wrapping opaque state, FFI shims). Every skip carries a comment
  explaining *why* — an unjustified skip is a review rejection.
- **Missed mutations are actionable, not aspirational.** A PR that
  introduces a missed mutation either adds a test or documents the
  reason inline. "We'll fix it later" is rejected.
- **Flaky tests break mutation testing.** A test that sometimes fails
  independent of the code under test marks every mutation as "caught" —
  worse than missing them. Fix flaky tests *before* adding them to the
  mutants run.
- **Regression gate on direction, not just level.** Baseline kill rate
  per crate is stored under `mutants-baseline/main/`. A drop > 2
  percentage points fails the PR even if absolute kill rate is still
  ≥ 80% — trend matters.

### What it's NOT for

- **`unsafe` blocks and `aya-rs` eBPF programs.** Mutations can produce
  code the verifier rejects, masquerading as "caught." Tier 2 and Tier 3
  cover these — do not run mutants against `crates/overdrive-bpf`.
- **Async scheduling logic.** Mutations to `select!` arms or future
  polling interact poorly with timing; DST is the right tool.
- **Generated code.** `#[derive(...)]`, `build.rs` output, proc-macro
  expansions — exclude by path in `.cargo/mutants.toml`.
- **Performance assertions.** A mutation that removes an optimisation
  may still pass correctness tests. Performance regressions are Tier 4's
  job.
- **`cargo xtask dst` / Tier 3 integration.** `cargo-mutants` reruns
  the unit suite per mutation under `--test-tool=nextest` (matches the
  project runner); DST and real-kernel tests are too slow for the
  per-mutation budget and are excluded from the mutants run. Doctests
  are also skipped by nextest and therefore by the mutants pass —
  doctest coverage is verified by the paired `cargo test --doc` step,
  not by mutation testing. This means mutation testing only covers
  code reachable from the unit-level suite — another reason unit-level
  invariants must be strong.

---

## Tier 2 — BPF Unit Tests

### Triptych shape

Each eBPF program ships three companions in `crates/overdrive-bpf/tests/`:

- `PKTGEN` — synthetic packet or syscall context.
- `SETUP` — populates the BPF maps the program reads.
- `CHECK` — drives `BPF_PROG_TEST_RUN` via `aya::Program::test_run()`,
  asserts on output / verdict / map mutations.

### Rules

- **Map state is cleared between sub-tests by default.** Persistent state
  across sub-tests is opt-in via `#[test_chain]` for cases that genuinely
  need staged setup (atomic-swap semantics, etc.). Default-persist was the
  Cilium choice; we chose default-isolate to match idiomatic Rust `#[test]`
  and avoid phantom failures.
- **Only applicable where `BPF_PROG_TEST_RUN` is the right mechanism** —
  XDP, TC. Sockops and BPF LSM move entirely to Tier 3 (the kernel does
  not expose `PROG_TEST_RUN` meaningfully for these).
- **Tier 2 does not prove hook attachment.** It proves program-level
  correctness against curated input. Attachment and invocation are
  Tier 3.

---

## Tier 3 — Real-Kernel Integration

### Kernel matrix

Every merge runs against:

- 5.10 LTS (floor — BPF LSM + kTLS + sockops jointly stable)
- 5.15 LTS (Ubuntu 22.04, Debian 12 backports, RHEL 9)
- 6.1 LTS (Debian 13)
- 6.6 LTS (Ubuntu 24.04)
- Current LTS
- `bpf-next` (soft-fail; nightly gate)

Adding a kernel is one line of YAML against `little-vm-helper`. Dropping a
kernel requires an ADR.

### Harness

- CI: `little-vm-helper` (OCI kernel images; same tooling Cilium, Tetragon,
  pwru use).
- Dev laptops: `virtme-ng` (~1s boot from a kernel tree).
- Entry point: `cargo xtask integration-test vm --cache-dir <CACHE_DIR>
  <KERNEL>...` — reuses aya's existing flow, do not fork.
- GitHub Actions runners work with `--qemu-disable-kvm`; self-hosted
  KVM-capable runners optional for latency budget.

### Running tests on macOS — Lima VM

`cargo nextest run` does not work on macOS. ProcessDriver, control-plane
cgroup management, eBPF programs, and every `#[cfg(target_os = "linux")]`
test surface require a real Linux kernel plus cgroup v2. macOS-side
`--no-run` catches type and wiring errors but not runtime or permission
issues — every shipped test must be exercised on Linux at least once before
merge, and the Lima VM is the canonical inner-loop path.

A pre-tool hook (`.claude/hooks/block-nextest-on-macos.ts`) enforces this
mechanically: bare `cargo nextest run` on macOS is blocked at the
tool-call boundary. The hook allows only `cargo xtask lima run -- cargo
nextest run ...` and the `--no-run` compile-check form.

**Where the VM is defined.** `infra/lima/overdrive-dev.yaml` describes
the project's standard dev VM (Ubuntu 24.04, kernel 6.8, cgroup v2,
KVM, full eBPF + BPF LSM toolchain, `cargo-nextest`, `cargo-mutants`).
The repo is virtiofs-mounted into the guest at the same path; no rsync,
no `git clone` inside the VM.

**Default invocation:**

```bash
limactl shell overdrive bash -lc \
  'cargo nextest run --workspace --features integration-tests'
```

The path inherits from `pwd` on the host because the working tree is
mounted at the same absolute path in the guest. `--features
integration-tests` is mandatory; without it, the Linux-gated tests are
skipped and the run signal is meaningless.

**Cgroup writes need root or delegation.** Tests that exercise the
workload-cgroup path (`overdrive-worker::ProcessDriver`, the
JobLifecycle convergence loop) `mkdir`
`/sys/fs/cgroup/overdrive.slice/...`. The Lima default user is
unprivileged and lacks delegation for that subtree, so the production
path returns `EACCES` and Pending allocs never reach Running. The
canonical inner-loop shape is:

- **`cargo xtask lima run` (canonical, 1:1 with CI)** — the wrapper
  defaults to running the test process as root inside the VM, the same
  permission surface CI's LVH harness uses. It re-injects `PATH` and
  `CARGO_TARGET_DIR` so cargo and its target dir continue to resolve
  under the `lima` user's home (where rustup is installed):

  ```bash
  cargo xtask lima run -- cargo nextest run --workspace --features integration-tests
  ```

  Equivalent under the hood to:

  ```bash
  limactl shell overdrive bash -lc \
    'sudo -E env "PATH=$PATH" "CARGO_TARGET_DIR=$CARGO_TARGET_DIR" \
     cargo nextest run --workspace --features integration-tests'
  ```

  Pass `--no-sudo` (`cargo xtask lima run --no-sudo -- <cmd>`) to run
  as the unprivileged `lima` user — for non-test commands or when you
  specifically want to observe the EACCES surface without privileged
  delegation. `cargo xtask lima shell` still drops you in as the
  `lima` user; use `sudo -i` inside if you want an interactive root
  shell.

There is no in-binary escape hatch. ADR-0034 removed the
`--allow-no-cgroups` flag (was structurally broken — leaked workloads
in the StopAllocation path — and rendered redundant by the Lima
wrapper above). Tests that exercise the `state: Terminated` shape
against a real workload run via `cargo xtask lima run --`; tests that
want to assert on the control-plane convergence shape with no real
process at all use `SimDriver` per the standard DST convention.

Do not paper over `EACCES` failures by removing the cgroup writes — the
production code path IS the cgroup writes.

**The `--no-run` macOS gate is necessary, not sufficient.** Every step's
quality gate includes `cargo nextest run --workspace --features
integration-tests --no-run` on macOS to catch compile errors before
shipping. That gate cannot detect convergence-loop bugs, race
conditions, missing match arms behind `#[cfg(target_os = "linux")]`,
permission shape issues, or any runtime invariant. A green `--no-run`
on macOS plus a green Lima run is the honest signal; either alone is
not.

**Tier 3 / Tier 4 stays on `cargo xtask integration-test vm` and
`cargo xtask xdp-perf`.** The Lima VM is the macOS dev convenience for
running the per-step integration suite during the inner loop. The
kernel-matrix tier 3 harness still runs on CI via LVH; do not collapse
the two.

**Mutation testing falls under the same rule.** Any `cargo xtask
mutants` invocation that includes `--features integration-tests` runs
through `cargo xtask lima run --` on macOS — see § "Mutation testing
(cargo-mutants)" → "Usage" for the full rationale. Without the prefix
the mutation run uses a degraded test signal (the `#[cfg(target_os =
"linux")]` surface is unreachable) and the kill-rate gate becomes
meaningless. The check is mechanical: does this command pass
`--features integration-tests`? If yes, prefix with `cargo xtask lima
run --`.

### Assertion rules

Assert on observable kernel side effects. Never on program internal
reachability ("the program took branch X").

Three observable layers:

- **Kernel-side state**
  - BPF maps: `bpftool map dump`
  - TLS ULP: `ss -K`
  - LSM decisions: BPF ringbuf event stream (the *event*, not "the program
    returned `EPERM` early")
- **Userspace state**
  - Structured flow events from the Overdrive telemetry ringbuf
- **Wire capture**
  - `tcpdump` on veth interfaces
  - Expected ciphertext (kTLS), expected forwarding (XDP SERVICE_MAP)

Counter-example (do not do this):

```rust
// WRONG — asserts the program reached a branch, not that the kernel
// invoked the hook.
assert_eq!(ebpf_program.last_action.load(Ordering::Relaxed), ACTION_DENY);
```

Correct form:

```rust
// The hook fires and the userspace observer sees the deny event.
let event = ringbuf.recv_within(Duration::from_secs(1))?;
assert_eq!(event.verdict, Verdict::Deny);
assert_eq!(event.lsm_hook, LsmHook::FileOpen);
```

### Mandatory test cases per hook

Every new eBPF program lands with the coverage below or it does not merge:

| Hook | Minimum coverage |
|---|---|
| XDP | Atomic map swap under load; zero-drop invariant across the update |
| TC | Egress redirection path through `SIDECAR_MAP` |
| sockops | ULP install verified via `ss -K`; handshake failure on wrong SVID |
| sockops + kTLS | Wire capture shows TLS 1.3 records |
| BPF LSM | Positive *and* negative case per policy bit (denied + allowed) |
| End-to-end | IntentStore write → Corrosion propagation → kernel verdict |

---

## Tier 4 — Verifier and Performance Gates

### Verifier complexity (`veristat`)

- Full BPF corpus compiled with worst-case feature flags, loaded into every
  matrix kernel.
- Baseline on `main`. PR fails if:
  - Any program exceeds its baseline instruction count by >5%.
  - Any program approaches the per-program complexity ceiling by >10%.
- Verifier behaviour changes across kernel releases. The only guard is
  loading the corpus into every kernel in the matrix. Do not rely on a
  single-kernel verifier-pass signal.

### XDP performance (`xdp-bench`)

- `xdp-trafficgen` → SUT → sink, two veth pairs inside an LVH VM.
- Baseline per-runner-class pps and p99 latency under
  `perf-baseline/main/`.
- PR fails if relative delta exceeds:
  - 5% pps regression
  - 10% p99 latency regression
- **Never gate on absolute numbers** — runner hardware varies enough to
  make absolute gates flaky. Deltas only.

### Second-opinion static analysis (PREVAIL)

- Nightly, non-blocking.
- Fails the build when PREVAIL disagrees with the kernel verifier's
  accept/reject decision.
- This defends against verifier bugs, not just program bugs.

---

## Fault injection catalogue

Every release exercises the fault classes below. The DST fault and its
real-kernel complement are written together — neither alone is sufficient.

| Class | DST (Tier 1) | Real kernel (Tier 3) |
|---|---|---|
| Network partition | `SimTransport.partition()` | `tc qdisc … netem loss 100%` on veth |
| Packet loss | `SimTransport` loss | `netem loss 5%` |
| Reordering | `SimTransport` reorder | `netem reorder 50% gap 3` |
| Latency | `SimTransport` delay | `netem delay 100ms 20ms` |
| Clock skew | `SimClock` offset | VM boot with offset `CLOCK_REALTIME` |
| Node crash | restart hook in turmoil host | `kill -9` the in-VM binary |
| Corrosion gossip stall | `SimObservationStore` stall | real Corrosion; pause peer event loop |
| Schema migration storm | `SimObservationStore` migration | additive migration against real Corrosion |
| Driver failure | `SimDriver` configured to fail | inject bad kernel image in CH |
| Policy eval timeout | inject `Llm`/`Regorus` hang | hang the real Regorus call |

The same catalogue drives the chaos engineering reconciler in production.
Tests and chaos share the fault definitions; a fault is specified once.

---

## CI topology

```
Per-PR (critical path ≈ 15 minutes):
  A1 cargo nextest run --workspace       unit + proptest, no BPF       (s)
  A2 cargo test --doc --workspace        rustdoc examples              (s)
  B  cargo xtask dst                     Tier 1                        (min)
  C  cargo xtask bpf-unit                Tier 2                        (min)
  D  cargo xtask integration-test vm     Tier 3, kernel matrix         (10 min)
  E  cargo xtask verifier-regress        Tier 4 — veristat             (min)
     cargo xtask xdp-perf                Tier 4 — xdp-bench            (min)
  F  cargo xtask mutants --diff origin/main
                                         diff-scoped (nextest per      (min)
                                         mutation); kill rate ≥ 80%

Nightly:
  G  Tier 3 + Tier 4 against bpf-next                                  soft-fail
  H  PREVAIL second-opinion analysis                                   soft-fail
  I  cargo xtask mutants --workspace     full corpus; trend tracking
  J  Long-run fault-injection soak with random netem profiles

Per-release:
  K  Full Tier 3 matrix on aarch64 (self-hosted Graviton runner)
```

---

## Scope boundaries

Explicitly out of scope:

- **Real hardware NIC drivers.** We run against virtio-net and veth in
  QEMU — the same envelope Cilium, Tetragon, and upstream BPF CI use. Real
  hardware validation lives in a per-release lab, not per-PR.
- **Kernel selftests.** We do not re-run `tools/testing/selftests/bpf`.
  That is the kernel's job. We rely on each supported kernel having passed
  its own selftests.
- **Production chaos as a CI substitute.** The chaos reconciler validates
  emergent production behaviour. It does not replace pre-merge gating.

---

## Adding a new test — which tier?

```
Logic bug under concurrency, timing, ordering, or partition?
    → Tier 1 (DST)

Pure function whose argument space exceeds a dozen hand-picked cases?
    → Property-based test (proptest)

Type-system property that must hold at compile time (non-substitutable
traits, banned parameter shapes)?
    → trybuild compile-fail test

Tests pass but not sure they actually assert on the behaviour?
    → cargo-mutants — close the kill-rate gap

Test mutates env vars / cwd / other process-global state?
    → `#[serial_test::serial(env)]` + RAII guard
      (see "Tests that mutate process-global state")

eBPF program-level correctness against curated input?
    → Tier 2 (BPF unit)

Does the program actually load, attach, and enforce on real kernels?
    → Tier 3 (integration)

Does a change bloat verifier complexity or regress XDP throughput?
    → Tier 4 (perf / verifier gates)
```

When in doubt, start with Tier 1 and promote upward. DST failures are the
cheapest to reproduce; real-kernel failures are the cheapest to trust.
