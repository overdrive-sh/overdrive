# Journey: Trust the Sim — Visual

**Feature**: `phase-1-foundation`
**Persona**: Ana, Overdrive platform engineer (distributed systems SRE background, primary author of control-plane logic).
**Goal**: run `cargo xtask dst` on a clean clone and trust that a red run is reproducible from the printed seed.

---

## High-level flow

```
  [Fresh clone]                                                     [Trust]
       |                                                               ^
       v                                                               |
  Step 1:  cargo xtask dst                                             |
           Green invariants in <3s                                     |
           SKEPTICAL --> CAUTIOUSLY OPTIMISTIC                         |
              |                                                        |
              v                                                        |
  Step 2:  cargo test -p overdrive-core                                |
           Newtype round-trip + LocalStore snapshot                    |
           FOCUSED --> CONFIDENT                                       |
              |                                                        |
              v                                                        |
  Step 3:  cargo xtask dst-lint (engineer inserts Instant::now())      |
           CI gate blocks with pointed error                           |
           CURIOUS --> REASSURED                                       |
              |                                                        |
              v                                                        |
  Step 4:  cargo xtask dst (real invariant failure)                    |
           Seed printed, reproduction command inline, same seed --> same
           ANXIOUS --> TRUSTING ---------------------------------------+
```

## Emotional arc

```
  trust
    ^
    |                                                      .  Step 4
    |                                          . Step 3    .
    |                              . Step 2   .            .
    |                 . Step 1    .          .             .
    |     .           .           .          .             .
    |    skeptical    cautious    confident  reassured    TRUSTING
    +---------------------------------------------------------> time
```

The arc is a **Confidence Building** pattern (whitepaper-mapped; see `nw-ux-emotional-design` "Confidence Building" curve). Each small green is a deposit. The **peak tension** is Step 4 — a real distributed bug forces engineers to decide whether they trust the harness. The **peak relief** is the same-seed reproduction landing at the same tick.

---

## Step 1 — Fresh clone, first DST run

```
$ cargo xtask dst
    Compiling overdrive-core v0.0.0
    Compiling overdrive-sim  v0.0.0
    Finished `dev` profile [optimized + debuginfo]
     Running DST harness (turmoil, 3-node cluster)

    seed: ${dst_seed}
    ticks/run: 60000 (simulated 60s @ 1ms)
    runs: 100

    [  1/100] leader_election_under_partition            ok
    [  2/100] intent_observation_isolation               ok
    [  3/100] local_store_snapshot_roundtrip             ok
    [  4/100] sim_observation_lww_converges              ok
    [  5/100] replay_equivalence_empty_workflow          ok
    ...
    [100/100] entropy_determinism_under_reseed           ok

    100 scenarios  ·  0 failures  ·  2.3s wall-clock
```

Key UX properties (CLI pattern guidance from `nw-ux-tui-patterns`):
- **First output within 100ms** — cargo compile messages provide immediate feedback.
- **Progress for long ops** — `[N/100]` counter, not a spinner. DST runs are bounded and countable.
- **Summary line last** — exit code 0 leaves the summary at the bottom of scrollback.

---

## Step 2 — Write a reconciler that uses the intent store

```
$ cargo test -p overdrive-core --test intent_store
    Finished `test` profile

running 7 tests
test newtype_roundtrip::job_id_fromstr_display ................ ok
test newtype_roundtrip::node_id_fromstr_display ............... ok
test newtype_roundtrip::spiffe_id_fromstr_display ............. ok
test local_store::put_get_delete_are_consistent ............... ok
test local_store::snapshot_roundtrip_is_bit_identical ......... ok
test local_store::bootstrap_from_snapshot_preserves_state ..... ok
test local_store::watch_fires_on_prefix_match ................. ok

test result: ok. 7 passed; 0 failed; 0 ignored
```

The engineer is using ordinary `cargo test` here; the point is that the same primitives that the DST harness exercises work for conventional unit tests too. `LocalStore` is the **real** redb store used in production, not a mock.

---

## Step 3 — CI lint gate catches a banned API

```
$ cargo xtask dst-lint
     Scanning overdrive-core for banned nondeterministic APIs...

error: nondeterministic API used in core crate
  --> crates/overdrive-core/src/reconciler.rs:42:21
   |
42 |     let now = std::time::Instant::now();
   |                     ^^^^^^^^^^^^^^^^^^
   |
   = note: core crates MUST go through the `Clock` trait.
           See `.claude/rules/development.md` — "Nondeterminism
           must be injectable". If `Clock` is missing a method
           you need, add it to the trait rather than bypassing it.

1 banned-API violation found. Lint gate FAILED.
```

Error-message design (from `nw-ux-tui-patterns` "Error Message Design"):
1. **What happened**: "nondeterministic API used in core crate".
2. **Why**: points at the exact file:line:column and the banned symbol.
3. **What to do**: names the trait (`Clock`), names the rule source (`development.md`), and tells the engineer to extend the trait rather than bypass it.

---

## Step 4 — Real failure reproduces from the seed

```
$ cargo xtask dst
    ...
    [ 37/100] leader_election_under_partition  FAILED

invariant violated: single_leader
  seed:   ${dst_seed}
  tick:   8743 (simulated t=8.743s)
  where:  turmoil host "node-0"
  cause:  2 leaders elected after partition heal

reproduce:
  cargo xtask dst --seed ${dst_seed} --only leader_election_under_partition
```

Running the reproduction command:

```
$ cargo xtask dst --seed ${dst_seed} --only leader_election_under_partition
    Same seed. Same trajectory.
    [ 1/1] leader_election_under_partition     FAILED (tick 8743)
```

**This is the emotional peak of the journey.** A red run is not a problem; a red run that does not reproduce is a catastrophe. The fact that the second run fails **at the same tick** is the moment the engineer starts trusting the harness as infrastructure rather than as documentation.

---

## Shared artifacts (visual summary)

```
dst_seed              -->  Step 1 output  -->  Step 4 output  -->  reproduction command
                                                                    (MUST be identical format)

invariant_name        -->  Step 1 list    -->  Step 4 failure line
                           (enum in overdrive-sim/invariants.rs is the single source of truth)

newtype_canonical     -->  Step 2 tests   -->  snapshot bytes (Step 2)
                           Display == FromStr-accepting == serde output
                           (if any drift, hashing diverges)

banned_api_list       -->  Step 3 error   -->  development.md  -->  CI gate config
                           (xtask constant is the SSOT; docs reference it)
```

## Failure modes (per step — fed into DISTILL)

| Step | Mode |
|---|---|
| 1 | Harness fails to compile because a core crate still directly uses `std::time::Instant::now()` |
| 1 | Harness hangs because a Sim* trait is not wired to a deterministic clock |
| 1 | Harness is non-deterministic — same seed produces different output on two runs |
| 2 | `FromStr` accepts garbage that `Display` cannot re-emit |
| 2 | `serde_json` round-trip produces different bytes than `rkyv` (hashes diverge) |
| 2 | snapshot → `bootstrap_from` → re-export is not bit-identical |
| 3 | Lint gate only scans some core crates, misses a new one |
| 3 | Lint message names a rule but does not name the trait to use |
| 3 | Gate is advisory, not blocking — CI passes anyway |
| 4 | Seed prints but second run diverges — a source of nondeterminism was missed |
| 4 | Failure output is a generic `assertion failed` with no invariant name |
| 4 | Reproduction command is missing (engineer has to guess the invocation) |

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial visual for phase-1-foundation DISCUSS wave. |
