# RED-classification PLAN — `mtls-intercept-install-fault-seam`

**Wave**: DISTILL (the PLAN) → DELIVER's RED phase (the actual run).
**Designer**: Quinn | **Date**: 2026-08-01 | **Feature**: GH #250

Per ADR-025 D2 the pre-DELIVER **fail-for-the-right-reason gate** becomes
DELIVER's RED-phase entry/exit gate. DISTILL authors the scenarios
(`test-scenarios.md`) and this plan; **DISTILL does not run the
classification** and **writes no file under `crates/`** this wave.

This feature is unusual and the difference is load-bearing: **three of its
thirteen scenarios have production code that already exists and already
works.** They are authored **GREEN**, and their falsification is a *litmus*,
not a scaffold panic. Getting this wrong in either direction — attaching
`#[should_panic(expected = "RED scaffold")]` to a test whose behaviour is
already implemented, or shipping a green test with no proof it can fail — is
the failure mode this document exists to prevent.

---

## 1. Two RED shapes, and which scenarios use which

| Shape | When it applies | Mechanism | Scenarios |
|---|---|---|---|
| **A — mutation/edit litmus** (authored GREEN) | The production behaviour **already exists**; the test is the missing *assertion*. There is nothing to scaffold. | Author the test green. Prove it can fail by making the pinned edit, observing RED, and reverting via `Edit` (never `git checkout`). Record the observed failure on the DELIVER step. | S-MIF-01, S-MIF-02, S-MIF-03 |
| **B — RED scaffold** (`#[should_panic(expected = "RED scaffold")]`) | The production surface the test names **does not exist yet**. | `.claude/rules/testing.md` § "RED scaffolds": `#[should_panic(expected = "RED scaffold")]` on the test with a `panic!("Not yet implemented -- RED scaffold (<id> / <one-line spec>)")` body; `todo!("RED scaffold: …")` on any production stub, gated `#[expect(clippy::todo, reason = "RED scaffold; lands GREEN in step NN")]`. | S-MIF-04..S-MIF-13 |

**Shape A is NOT a licence to skip falsification.** A green test that was
never observed failing is indistinguishable from a vacuous one — which is
precisely the trap architecture.md § 6.5 warns about (a 100 % kill rate over a
one-mutant set). The litmus below is mandatory and its output is recorded on
the step.

### Why S-MIF-01/02/03 are shape A

DFS-0a, verified at DESIGN review iteration 1 and re-verified here:
`fail_closed_on_mtls_install` is an `async fn` with **no `pub`** at
`crates/overdrive-control-plane/src/action_shim/mod.rs:413`; the file's own
`#[cfg(test)] mod tests` at `:1841` already reaches parent items via
`use super::{…}` at `:1869`; all eight arguments are default-lane
constructible with no I/O; and `cause` is constructible cross-crate because
`#[non_exhaustive]` on `MtlsInterceptInstallError` is **enum-level only**
(it blocks exhaustive *matching*, not *construction*).

**Every symbol S-MIF-01/02/03 name exists in `crates/` today.** There is no
`ImportError`, no missing type, no stub to write. Attaching a RED scaffold
would assert "not yet implemented" about code that is implemented — a false
RED, and the exact BROKEN-vs-RED confusion the Red Gate Snapshot exists to
separate.

---

## 2. The mandatory litmus for shape A (DELIVER step 01)

Three edits, each applied → observe RED → **revert with `Edit`** (never
`git checkout --` — the destructive-git-ops hook blocks it, and per project
memory that command is not the recovery path here). Record the observed
failing test name + assertion for each on the step.

| Litmus | Edit | Must turn RED | Proves |
|---|---|---|---|
| **L-1 — the suppressed mutant** | Replace the whole body of `fail_closed_on_mtls_install` with `Ok(())` | **S-MIF-01** (all 6 cases; A-2 fires first) | The test kills the mutant `.cargo/mutants.toml` currently suppresses. **This is the falsification the entire un-suppression rests on.** |
| **L-2 — the best-effort stop** | `let _ = driver.stop(handle).await;` → `driver.stop(handle).await?;` | **S-MIF-02** | A vanished workload no longer aborts the handler before the `Failed` row is written — the un-alarmed exclusion-mechanism failure (architecture.md § 1). |
| **L-3 — the write/emit ordering** | `obs.write(..).await?;` → `let _ = obs.write(..).await;` | **S-MIF-03** | The rejection is reported, not swallowed, and no lifecycle transition is announced without a durable row behind it. |

**Do NOT rely on `cargo mutants` to supply L-1's evidence.** Per
architecture.md § 6.5 and project memory
(`reference_cargo_mutants_blind_to_spawn_blocking_and_saturating_add`),
cargo-mutants can generate **zero** mutants for a load-bearing arm, and a
100 % file-scoped kill rate can be vacuous. The manual litmus stands
regardless of what the tool generates. The tool run (§ 4) is *additional*, not
a substitute.

### The A-6' litmus — the one that justifies the port (DELIVER step 04)

The whole design rests on **one** leg (DFS-1): the port makes the *call-site
ordering* testable at all. If S-MIF-04/05 go green without ever being shown to
fail on a reordering, the port bought nothing and the feature is unfalsified.

| Litmus | Edit | Must turn RED | Proves |
|---|---|---|---|
| **L-4 — the gate-release ordering** | In the `StartAllocation` arm, move the `if let Some(handle) = &handle_opt { driver.release_for_exit_emission(handle); }` block (`mod.rs:1319`) to **above** the mTLS guard's `return` (`:1307`) | **S-MIF-04**, assertion **A-6'** | A now-`Failed` allocation would release its exit watcher. This assertion — and only this one — dies on the reordering, and **it survives T1 entirely** (a helper-level test structurally cannot reach it). |
| **L-5 — the restart arm** | The same move in the `RestartAllocation` arm (`:1519` above `:1507`) | **S-MIF-05**, assertion **A-6'** | The second arm is independently defended (OQ-6's "future divergent edit to one block"). |

L-4/L-5 run under Lima as root, same invocation as the tests themselves.
**If A-6' does not go RED under L-4, stop and surface a blocker** — it means
the assertion is not observing the release, and the port's sole justification
is unproven.

---

## 3. Expected RED reason per scenario, and the DELIVER step each goes GREEN

DELIVER ordering is pinned by **DFS-6 / architecture.md § 9**: T1 + both
suppression deletions land as **one** step, **first**, before the port
extraction. It is independently valuable, independently gated, needs no
production change, and de-risks the mutation contract from everything that
follows. Bundling it with the port would make the gate's green depend on the
port's correctness.

| Scenario | Lane | RED shape | Expected RED reason | GREEN in step |
|---|---|---|---|---|
| S-MIF-01 | 1 / default | **A — litmus L-1** | n/a — authored GREEN. Under L-1 the failure is a plain assertion failure (A-2: no superseding `Failed` row), **not** a scaffold panic. | **01** |
| S-MIF-02 | 1 / default | **A — litmus L-2** | n/a — authored GREEN. Under L-2: assertion failure (no `Failed` row). | **01** |
| S-MIF-03 | 1 / default | **A — litmus L-3** | n/a — authored GREEN. Under L-3: assertion failure (`Ok` returned where `Err(ShimError::Observation)` expected). | **01** |
| S-MIF-04 | int / Lima+root | **B — scaffold** | `MISSING_FUNCTIONALITY` — `mtls_intercept_port::MtlsIntercept`, `SimMtlsIntercept`, `SimInterceptFault` and the 4-arg `MtlsInterceptWorker::new` do not exist. Plus **litmus L-4** once green. | **04** |
| S-MIF-05 | int / Lima+root | **B — scaffold** | same, plus **litmus L-5** once green. | **04** |
| S-MIF-06 | 1 / default | **B — scaffold** | `MISSING_FUNCTIONALITY` — `SimMtlsIntercept` / `SimInterceptFault` absent. | **03** |
| S-MIF-07 | 1 / default | **B — scaffold** | same (standing-fault lifetime unimplemented). | **03** |
| S-MIF-08 | 1 / default | **B — scaffold** | same (`clear_faults` absent). | **03** |
| S-MIF-13 | 1 / default | **B — scaffold** | same (the three independent fault slots absent). | **03** |
| S-MIF-09 | int / Lima+root | **B — scaffold** | `MISSING_FUNCTIONALITY` — `HostMtlsIntercept` / `SimMtlsIntercept` / the `MtlsIntercept` trait absent. | **05** |
| S-MIF-10 | int / Lima+root | **B — scaffold** | same. | **05** |
| S-MIF-11 | int / Lima+root | **B — scaffold** | same (`InterceptGuard` + `Box<dyn InterceptGuard>` returns absent). | **05** |
| S-MIF-12 | int / Lima+root | **B — scaffold** | same. | **05** |

### The five DELIVER steps

| Step | Content | Gate |
|---|---|---|
| **01** | S-MIF-01/02/03 (authored GREEN) + **delete** the `.cargo/mutants.toml` `exclude_re` entry `"fail_closed_on_mtls_install"` (`:592-615` incl. its comment) + **delete** the source-site `// mutants: skip` block (`action_shim/mod.rs:403-412`). **No production change.** | Litmus L-1/L-2/L-3 each observed RED then reverted. `cargo mutants --list` scoped to the file, with the mutants generated inside the function **recorded on the step**. The scoped gate re-run (§ 4) at **100 % for the function** — reported as vacuous if the generated set is a single mutant. |
| **02** | The port: `crates/overdrive-worker/src/mtls_intercept_port.rs` (`InterceptGuard`, `MtlsIntercept`, `HostMtlsIntercept`), the `pub mod` registration, the `MtlsInterceptWorker` 4th mandatory param + guard-type widening to `Box<dyn InterceptGuard>`, the **four adjacent doc fixes** (`:467` intra-doc link, `:28` module prose, `:260-263` and `:269-273` the two `AllocIntercept` guard-field docstrings), the `run_server` wiring (**wire only, no gate**) + the step-(4) "three → four ports" comment, and **all 9 non-production call sites** passing `Arc::new(HostMtlsIntercept::new())`. | **Behaviour-preserving refactor — no new scenario goes GREEN here.** The gate is: the mandatory 4th `new()` parameter is compiler-enforced at every call site; the **existing** Tier-3 suite (`start_alloc_installs_both_tproxy.rs`, `bidirectional_walking_skeleton.rs`, `inbound_tproxy_harness.rs`, `outbound_enforce_substrate_asymmetry.rs`, `alloc_netns_lifecycle.rs`) stays green; step 01's mutation result is unaffected. |
| **03** | `crates/overdrive-sim/src/adapters/mtls_intercept.rs` (`SimInterceptFault`, `SimMtlsIntercept`, the private `InertGuard`), the `adapters/mod.rs` module decl **and** `pub use`, the `overdrive-worker.path` dep in `overdrive-sim/Cargo.toml`. | S-MIF-06/07/08/13 GREEN, default lane. |
| **04** | `crates/overdrive-control-plane/tests/integration/mtls_install_fail_closed.rs` + its `tests/integration.rs` declaration. | S-MIF-04/05 GREEN under Lima+root. **Litmus L-4 and L-5 each observed RED then reverted** — mandatory; this is the port's sole justification. |
| **05** | `crates/overdrive-worker/tests/integration/mtls_intercept_equivalence.rs` + its declaration. | S-MIF-09/10/11/12 GREEN under Lima+root, both adapters. |

Steps 04 and 05 are separable but may be merged if DELIVER prefers a single
Lima round-trip. Steps 01 and 02 **must not** be merged (DFS-6).

---

## 4. The scoped mutation re-run (step 01)

```
cargo xtask lima run -- cargo xtask mutants --diff origin/main \
  --features integration-tests \
  --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/action_shim/mod.rs
```

- Run **backgrounded** (`.claude/rules/testing.md` § "Mutation testing is the
  exception"). Do not `pkill`; do not add a post-run `git checkout`.
- Read the **guest** `target/xtask/mutants-summary.json` — the host artifact is
  stale on macOS.
- **Obligation: 100 % of the mutants inside `fail_closed_on_mtls_install`**,
  not ≥ 80 % — the `exclude_re` entry is a bare **function-name anchor**, so
  deleting it un-suppresses every mutant in the function.
- **Enumerate before claiming.** `cargo mutants --list` first; record the
  actual set. If it is a single whole-body mutant, say so and report the 100 %
  as **vacuous** — A-8/A-9/A-10 rest on the #248 forward-carry bug class, not
  on mutation coverage.
- If any mutant survives with only T1, **surface a blocker**. It is not a
  licence to re-add the suppression, and not a licence to lean on T2 (which
  does not exist yet at step 01).

---

## 5. What the gate FAILS on

DELIVER must fix the **test**, not the production code, if any scenario fails
as:

- **`IMPORT_ERROR` / unresolved type** on a shape-B scenario whose scaffold was
  not materialised (`MtlsIntercept`, `InterceptGuard`, `HostMtlsIntercept`,
  `SimMtlsIntercept`, `SimInterceptFault`, the 4-arg
  `MtlsInterceptWorker::new`). BROKEN, not RED.
- **`IMPORT_ERROR` on a shape-A scenario** — this would mean a symbol
  S-MIF-01/02/03 name is missing, which contradicts DFS-0a. **Stop and surface
  a blocker**: either the design's verified reachability claim is wrong, or the
  test reached for a symbol the design did not pin.
- **`FIXTURE_BROKEN` / `SETUP_FAILURE`** — the Lima fixture refuses to boot; a
  leaked netns / veth / `nft` rule / cgroup from a prior run; the
  `integration-tests` feature absent so the binary does not compile. Sweep
  leaked Lima state before re-running (`.claude/rules/testing.md` § leaked
  workload cgroups; `.claude/rules/debugging.md` § leftover XDP attachments;
  project memory `reference_pre_push_flaky_foundational_crate_lima_cleanup`).
  For S-MIF-04/05/09..12 specifically: the `NetnsGuard` pre-sweep and the
  `nft`-ruleset sweep are **mandatory**, not optional.
- **`SETUP_FAILURE` masquerading as a skip** — S-MIF-04/05/09..12 are
  `is_root()`-gated and **skip** on an unprivileged host. A run that skips
  every one of them is **not** a pass. The merge signal is the Lima+root run;
  a macOS-host run that skips them proves nothing about this feature's
  integration half.
- **`WRONG_ASSERTION` / `OBSERVABLE_NOT_AT_PORT`** — an assertion reading a
  private field instead of a declared Universe entry. Each scenario in
  `test-scenarios.md` declares its Universe explicitly (obs-store rows, `Driver`
  call records, `LifecycleEvent`s, the returned `Result`,
  `NetSlotAllocator::snapshot()`, `TcpListener::local_addr()`), so this should
  not arise. Two specific traps:
  - reading `SimMtlsIntercept`'s `Mutex<Option<SimInterceptFault>>` slots
    directly instead of observing the trait method's `Result`;
  - asserting *"exactly one `nft` rule exists"* through the trait in S-MIF-12
    — that clause is substrate, unobservable through `MtlsIntercept`, and
    belongs to `HostMtlsIntercept`'s own Tier-3 obligations. Asserting it at
    the trait level re-introduces the § 4.1 contract defect DFS-7 fixed.
- **A green suite with no litmus recorded.** Shape A without L-1/L-2/L-3, or
  S-MIF-04/05 without L-4/L-5, is an unfalsified pass. Reject.

---

## 6. Scaffold clippy discipline (shape B only)

Per project memory `feedback_distill_scaffold_clippy_discipline` and
`feedback_panic_format_string_brace_escaping`:

- Inject the file-level `#![allow(...)]` / `#[expect(clippy::todo, reason =
  "RED scaffold; lands GREEN in step NN")]` block **at scaffold creation**, not
  after a lefthook fail loop. `expect`, never `allow` — it self-removes when
  the scaffold goes GREEN.
- **No `{` / `}` in `panic!` format strings.** Write
  `TransparentListener ( errno )`, not `TransparentListener { errno }` — the
  `#[should_panic(expected = "RED scaffold")]` matcher does not care about
  brace shape, but the format-string parser does.
- Discover pending scaffolds with
  `grep -rn 'should_panic.*RED scaffold' crates/`. After step 05, **zero**
  should remain for this feature.
- `#[ignore]` is **not** used anywhere in this feature. The blocker for every
  shape-B scenario is "the production surface does not exist yet", which is
  `#[should_panic(expected = "RED scaffold")]` territory. The `is_root()`
  early-return is a runtime skip, not an `#[ignore]`.

---

## 7. Prerequisite before the first commit

`crates/overdrive-dataplane`'s `build.rs` hard-fails without
`target/bpf/overdrive_bpf.o`, and any commit touching `overdrive-core` /
`overdrive-control-plane` triggers `nextest-affected` across it. Run
`cargo xtask lima run -- cargo xtask bpf-build` before the step-01 commit
(project memory `feedback_bpf_object_prereq_for_trybuild`). Step 02 touches
`overdrive-worker` and `overdrive-sim`, so the same prerequisite applies
there.
