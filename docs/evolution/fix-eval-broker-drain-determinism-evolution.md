# Evolution: fix-eval-broker-drain-determinism

**Date**: 2026-04-25
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Wave shape**: bugfix (RCA → /nw-deliver, single-phase 7-step roadmap)
**Status**: shipped, all DES phases EXECUTED/SKIPPED with PASS verdict

---

## Defect

`EvaluationBroker::drain_pending` and `ReconcilerRuntime.reconcilers` in
`overdrive-control-plane` were backed by `std::collections::HashMap`, whose
default `RandomState` hasher is per-process random-seeded. Two seeded DST runs
with identical scenario inputs would therefore produce divergent dispatch
orderings the moment either map held two or more distinct keys at iteration
time, silently violating the K3 *seed → bit-identical trajectory* property
documented in `.claude/rules/testing.md` § *Sources of Nondeterminism* and
in whitepaper §21.

For Phase 1's single-reconciler topology (`noop-heartbeat`) the bug was
benign — a one-key map drains trivially. With Phase 2+ multi-reconciler
scenarios any DST invariant that observed dispatched-order or per-tick
accumulated-action lists would have gone red on every fresh `cargo xtask
dst` invocation, with no way to reproduce from the printed seed.

## Root cause

The project's nondeterminism-injection contract (`testing.md` §"Sources of
Nondeterminism") enumerates eight injectable boundaries — `Clock`,
`Transport`, `Entropy`, `Dataplane`, `Driver`, `IntentStore`,
`ObservationStore`, `Llm` — but omits *ordered-collection iteration* as a
nondeterminism source. There was no dst-lint clause banning bare
`std::collections::HashMap` from `core`-class crates the way
`std::time::Instant::now`, `tokio::net`, and `rand::thread_rng` are
already banned. The defect was therefore inevitable: `HashMap` is the
idiomatic Rust default, and nothing in the codebase nudged the author
toward `BTreeMap`.

Full RCA chain (preserved in feature workspace, archived with this
finalize):

1. `HashMap::drain()` order is per-process random.
2. `EvaluationBroker.pending` declared `HashMap` — transcribed verbatim
   from ADR-0013 §8 spec text without the spec text constraining the
   underlying map type.
3. DST harness `drive_broker_collapse` only exercised a single key,
   making the drain-order property invisible to the existing gate.
4. `BrokerCountersSnapshot` invariant only asserted on aggregated `u64`
   counters, never on the dispatched-order vector.
5. The structural gap — no rule, no dst-lint clause — made the class
   silent to every existing review pass.

## Decision

**`BTreeMap` over `IndexMap` or sort-and-collect.** Drain order should be
a pure function of the keys currently held, not of upstream submit
ordering. `BTreeMap` is the established project idiom — see
`crates/overdrive-sim/src/adapters/observation_store.rs:215-218`, which
documents exactly this choice for the same reason. `indexmap` is not a
workspace dependency, and pulling in a third crate to fix a
two-call-site bug would have been disproportionate. Sort-and-collect at
the drain boundary would have left the iteration nondeterminism in
place wherever a future caller used `.iter()` instead of
`.drain_pending()` — `BTreeMap` makes the property structural rather
than discretionary.

## Scope landed

1. **Production fix** — `EvaluationBroker.pending` and
   `ReconcilerRuntime.reconcilers` swapped to `BTreeMap`. `Ord` /
   `PartialOrd` derived on `ReconcilerName` and `TargetResource` (both
   newtypes wrap `String` with strict character set; `Ord` on the inner
   `String` is total and matches `Display` exactly). Obsolete "order is
   unspecified, callers should sort" disclaimer in `reconciler_runtime.rs`
   and the now-redundant `reconcilers.sort()` in the cluster status JSON
   handler both deleted.
2. **DST harness strengthening** — `drive_broker_collapse_multi_key`
   added to `crates/overdrive-sim/src/harness.rs` exercising five
   distinct keys, making drain-order visible to the gate.
3. **New DST invariant** — `BrokerDrainOrderIsDeterministic` registered
   in the sim catalogue. Runs the real `EvaluationBroker` twice with
   identical multi-key submit sequences and asserts the drained vec is
   bit-equal across runs. Without the BTreeMap fix this fails
   almost-always; with the fix it passes deterministically.
4. **Project-wide rule** — `.claude/rules/development.md` § *Ordered-collection
   choice* added, naming `BTreeMap` as the default for `core` and
   control-plane hot paths whose iteration order is observed and
   requiring a `// dst-lint: hashmap-ok <reason>` comment for any
   `HashMap` use in a `core`-class crate.
5. **dst-lint clause** — `xtask/src/dst_lint.rs` extended to scan
   `core`-class crates for bare `std::collections::HashMap` /
   `HashSet` and reject the file at PR time unless the use site carries
   the justification comment.
6. **Acceptance fixtures** — file-local lint extension in
   `eval_broker_collapse.rs` to ban `std::collections::HashMap` import
   from `eval_broker.rs`; new `runtime_registers_noop_heartbeat.rs`
   acceptance test for the runtime-side BTreeMap swap; xtask blessed
   catalogue entry for `broker-drain-order-is-deterministic`.

## Quality gate results

| Gate | Result |
|---|---|
| Phase 3.5 post-merge integration: `cargo nextest run --workspace` | PASS — 565 / 565 |
| Phase 3.5 doctests: `cargo test --doc --workspace` | PASS — 7 / 7 |
| Phase 3.5 DST: `cargo xtask dst` | PASS — 10 / 10 invariants, seed `7011461192756685183` |
| Phase 3.5 lint: `cargo clippy --workspace --all-targets -- -D warnings` | PASS — clean |
| Phase 3.5 dst-lint: `cargo xtask dst-lint` against real workspace | PASS — clean |
| Phase 4 adversarial review | APPROVED — no actionable issues |
| Phase 5 mutation: `overdrive-control-plane` (per-PR, diff-scoped) | PASS — 90.9% kill rate (10 / 11 production mutants) |
| Phase 5 mutation: `overdrive-sim` (per-PR, diff-scoped) | PASS — 100.0% kill rate (22 / 22 production mutants) |
| Phase 6 integrity: `python3 -m des.cli.verify_deliver_integrity` | PASS — all 7 steps complete trace |

The single missed mutant on `overdrive-control-plane` was on
`ReconcilerRuntime::reconcilers_iter`, a pre-existing getter; the
mutation is a structural artefact of the public API surface and
predates this feature. See *Follow-ups* below.

## Files modified

| File | Change |
|---|---|
| `crates/overdrive-core/src/reconciler.rs` | `Ord` / `PartialOrd` derived on `ReconcilerName` and `TargetResource` |
| `crates/overdrive-control-plane/src/eval_broker.rs` | `pending: HashMap` → `BTreeMap`; import swap |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs` | `reconcilers: HashMap` → `BTreeMap`; deleted obsolete sort-disclaimer |
| `crates/overdrive-control-plane/src/handlers.rs` | Removed redundant `reconcilers.sort()` in cluster status JSON handler |
| `crates/overdrive-control-plane/tests/acceptance/eval_broker_collapse.rs` | Added `drain_pending_is_deterministic_across_two_brokers` regression test; extended file-local lint to ban `HashMap` import |
| `crates/overdrive-control-plane/tests/acceptance/runtime_registers_noop_heartbeat.rs` | New acceptance test covering runtime-side BTreeMap swap |
| `crates/overdrive-sim/src/harness.rs` | New `drive_broker_collapse_multi_key` exercising five distinct keys |
| `crates/overdrive-sim/src/invariants/evaluators.rs` | New `evaluate_broker_drain_order_is_deterministic` running broker twice with same submit sequence |
| `crates/overdrive-sim/src/invariants/mod.rs` | Registered `BrokerDrainOrderIsDeterministic` invariant variant |
| `crates/overdrive-sim/tests/invariant_roundtrip.rs` | Roundtrip coverage for the new variant |
| `xtask/src/dst_lint.rs` | New clause: scan `core`-class crates for bare `std::collections::HashMap` / `HashSet`; require `// dst-lint: hashmap-ok` justification |
| `xtask/tests/acceptance/dst_lint_banned_apis.rs` | Acceptance fixtures covering both bare-HashMap reject and justified-HashMap accept paths |
| `xtask/tests/acceptance/dst_clean_clone_green.rs` | Adjusted to coexist with the new clause |
| `xtask/tests/acceptance/dst_harness_smoke.rs` | Added `broker-drain-order-is-deterministic` to blessed catalogue |
| `.claude/rules/development.md` | New section *Ordered-collection choice*; documented `// dst-lint: hashmap-ok` marker comment syntax |

## Forward-looking

The dst-lint clause delivered in step 01-07 makes this defect class
*structurally unrepeatable* in any future `core`-class crate: bare
`std::collections::HashMap` / `HashSet` will fail the gate at PR time
unless the use site carries an explicit `// dst-lint: hashmap-ok
<reason>` justification, which a reviewer can then push back on against
the rule. The canonical specification for the choice is
`.claude/rules/development.md` § *Ordered-collection choice*; that
document is the load-bearing artefact, not this evolution doc.

`overdrive-control-plane` is itself an `adapter-host` crate, not
`core` — the new clause does not retro-flag the two HashMap call sites
that this fix replaced (they are gone), but it WILL prevent any future
`core`-class crate from introducing the same shape. Sim-internal
HashMaps in `crates/overdrive-sim/src/adapters/` remain allowed (sim
crates are `adapter-sim`, not `core`).

## Follow-ups (NOT regressions in this feature's delivery)

1. **`ReconcilerRuntime::reconcilers_iter` missed mutant** — a single
   missed mutation on a pre-existing public getter on
   `ReconcilerRuntime`. The getter exists for the cluster status JSON
   handler and is exercised at the integration tier rather than at the
   per-method unit tier, so the per-PR diff-scoped mutants run cannot
   kill it. This is a pre-existing test gap on the runtime's public API
   surface; the feature itself ships at 90.9% (above the 80% gate
   threshold) and the runtime-side BTreeMap behaviour is independently
   covered by `runtime_registers_noop_heartbeat.rs`. File a follow-up
   to add a unit-tier test that exercises `reconcilers_iter` directly,
   or accept the gap and skip-justify per `testing.md` § *Mutation
   testing — Rules*.
2. **`xtask` mutants wrapper edge case** — `cargo xtask mutants
   --diff origin/main --package xtask --file xtask/src/dst_lint.rs`
   produced "No mutants to filter" and then crashed parsing a missing
   `outcomes.json`. The wrapper handles the empty-mutant case
   incorrectly when `--file` is the sole filter. Separate concern from
   this feature; file a follow-up against the xtask wrapper.

## Cross-references

- ADR-0013 §8 (cancelable-eval-set semantics) — *unchanged*. The §8 spec
  text says nothing about HashMap vs BTreeMap; the underlying map type
  is below the ADR's specification surface. This fix preserves the
  cancelable-eval-set contract.
- Whitepaper §21 — *Deterministic Simulation Testing*; the K3
  reproducibility property (seed → bit-identical trajectory).
- `.claude/rules/testing.md` § *Sources of Nondeterminism* — the
  injectable-trait contract this defect violated.
- `.claude/rules/development.md` § *Ordered-collection choice* — the
  new project-wide rule that closes the structural gap.
- `crates/overdrive-sim/src/adapters/observation_store.rs:215-218` —
  the precedent for the BTreeMap idiom this fix adopted.

## Commits (chronological)

| SHA | Step | Title |
|---|---|---|
| `8cf9119` | 01-01 | `fix(eval-broker): switch pending map to BTreeMap for deterministic drain order` |
| `86dd2e6` | 01-03 | `fix(reconciler-runtime): switch reconcilers map to BTreeMap; drop redundant sort in cluster status handler` |
| `9bc7a1c` | 01-04 | `test(dst-harness): add multi-key drive_broker_collapse_multi_key for drain-order coverage` |
| `bc95506` | 01-05 | `test(dst): add BrokerDrainOrderIsDeterministic invariant + evaluator` |
| `54d2d7f` | 01-05 fixup | `test(xtask): add broker-drain-order-is-deterministic to blessed catalogue` |
| `e50146a` | 01-06 | `docs(rules): add 'Ordered-collection choice' section to development.md` |
| `cbff703` | 01-07 | `feat(dst-lint): ban bare HashMap/HashSet in core crates; require // dst-lint: hashmap-ok justification` |
| `019341c` | 01-06 enhancement | `docs(rules): document // dst-lint: hashmap-ok marker comment syntax` |

(One additional commit on the same branch — `86de926` *docs(rules):
require .nwave/des-config.json to always be committed when modified* —
landed interleaved with this work but is a separate concern unrelated
to the broker-drain-determinism fix and is not part of this feature.)
