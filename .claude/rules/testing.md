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

### Mechanics

Add an opt-in feature to the owning crate's `Cargo.toml`:

```toml
[features]
# Opt-in gate for slow / real-infra tests in this crate. Off by
# default so `cargo nextest run --workspace` stays under the 60s
# slow-test budget; CI exercises the gate in a dedicated
# `integration` job.
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

- **Do NOT** set `run_in_background: true` on a test command and then
  poll with `tail`, `cat`, `wait`, or sleep loops against the output
  file. Each poll burns a turn, the harness blocks long `sleep`s, and
  you lose the structured output view that the direct tool result
  gives you.
- **Do NOT** redirect test output to a temp file and `tail` it. The
  tool already captures stdout+stderr. Piping to `tail -N` in the
  command itself is fine if you know you only want the last N lines —
  but run it synchronously, not in the background.
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

- **Scoped per PR.** `cargo xtask mutants --in-diff origin/main` runs
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
  F  cargo xtask mutants --in-diff       diff-scoped (nextest per      (min)
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

eBPF program-level correctness against curated input?
    → Tier 2 (BPF unit)

Does the program actually load, attach, and enforce on real kernels?
    → Tier 3 (integration)

Does a change bloat verifier complexity or regress XDP throughput?
    → Tier 4 (perf / verifier gates)
```

When in doubt, start with Tier 1 and promote upward. DST failures are the
cheapest to reproduce; real-kernel failures are the cheapest to trust.
