# Upstream issues — phase-1-foundation (DELIVER)

Known residuals surfaced during the Phase 5 (mutation testing) cross-cutting
run that are **intentionally not addressed in Phase 1**. Each entry states
the gap, why it's acceptable here, and where it lands next.

---

## 1. `SimObservationStore::PeerState::dominates_for_merge` tiebreak — 4 missed mutants

**Location**: `crates/overdrive-sim/src/adapters/observation_store.rs:200-204`

**Missed mutations**:
- `200:37` — `&&` → `||` in the tiebreak conjunction
- `200:22` — `==` → `!=` on the timestamp equality guard
- `200:58` — `==` → `!=` on the peer-id equality guard
- `204:20` — delete `!` in the precedence check

**Why**: `dominates_for_merge` runs the LWW tiebreak when two rows share a
logical timestamp. The 04-02 acceptance tests exercise the higher-timestamp
path heavily, but the equal-timestamp branch is only hit by the specific
tiebreak scenario in 04-02 and the LWW proptest in 04-03 — the proptest
generator rarely synthesises exact timestamp collisions, so four of the
eight branches in `dominates_for_merge` stay uncovered by the current
generator.

**Not a Phase 1 blocker because**: the tiebreak is deterministic (peer-id
wins) and the LWW convergence proptest still holds at any delivery order.
A bad tiebreak rule would eventually surface as a convergence failure in
production even if these specific mutants slip through.

**Where it lands**: Phase 2 convergence-engine work will add reconciler-
driven tests that exercise equal-timestamp scenarios under real load; the
tiebreak branches will be covered there. If Phase 2 doesn't naturally hit
them, add a targeted tiebreak-specific proptest generator in the same slice.

---

## 2. `evaluate_sim_observation_lww` internal write/compare — 3 missed mutants

**Location**: `crates/overdrive-sim/src/invariants/evaluators.rs:302,329,330`

**Missed mutations**:
- `302:20` — `>=` → `<` on the write-count guard
- `329:38` — `+` → `*` on the peer-id arithmetic producing the second write
- `330:30` — `==` → `!=` on the value byte equality

**Why**: the evaluator drives two concurrent writes from different peers to
give LWW something to resolve, then delegates the convergence check to
`check_lww_convergence` from step 04-03. The missed mutants are in the
write-setup code, not the invariant logic itself. The setup is tested
indirectly through the harness smoke tests but no unit test asserts on the
exact bytes written.

**Not a Phase 1 blocker because**: the invariant result (converged / not)
is what CI gates on, and that is covered. A miswritten setup would either
(a) still converge (passing — no regression) or (b) trivially fail the
invariant on every seed (catastrophic — would show in CI immediately).

**Where it lands**: Phase 2 adds real reconcilers that write through the
observation store; the evaluator's setup code becomes a test subject on
its own at that point. If warranted, add a unit test asserting exact
bytes in the same commit that introduces the reconciler.

---

## 3. `overdrive-sim::real::CountingOsEntropy` — 5 missed mutants (feature-gated, excluded from scope)

**Location**: `crates/overdrive-sim/src/real/mod.rs:132,138,143`

**Why not a gap**: the `real/` module is compiled only under
`--features real-adapters`; the per-PR mutation run does not enable the
feature, so these mutants are tested in a build where the code is dead.
They'll be covered when Phase 2+ adds production consumers of
`SystemClock`/`OsEntropy`/`TcpTransport` under
`overdrive-node` / `overdrive-control-plane`.

Listed here for completeness; not classified as a Phase 1 gap.

---

## Summary

- Platform-code kill rate, excluding `xtask/**` (rule 6 in `.cargo/mutants.toml`)
  and excluding feature-gated `real/` code: ≈ 95.5% (149 / 156).
- The two actionable residuals (items 1 and 2) live in `overdrive-sim`, do
  not affect any platform correctness claim, and have natural homes in
  Phase 2 work.
- No Phase 1 rework required.
