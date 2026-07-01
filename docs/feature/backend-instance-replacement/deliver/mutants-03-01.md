# Mutation evidence — step 03-01 (A1 pump half-close-forward)

**Step**: `03-01` — the A1 directional clean-close half-close forward
(ADR-0070 amendment 2026-07-01). On a `PumpExit::Graceful` source EOF
that is NOT a deliberate `teardown`, each pump forwards the peer FIN to
its OPPOSING leg via `shutdown(dst_fd, SHUT_WR)` before `mark_exited`,
so a client holding an in-flight read fails fast (the S-DBN-CHURN fix)
instead of hanging to `TCP_USER_TIMEOUT`. `mark_exited` still fires the
(B) self-teardown ONLY on `TransportDeath` (unchanged).
**Surface under gate**:
`crates/overdrive-dataplane/src/mtls/splice.rs` — the
`forward_half_close_if_source_eof` helper (the `shutdown(SHUT_WR)`
forward + its `exit == Graceful && !state.stop` guard) and the two
production call sites that invoke it on their terminal exit
(`run_decrypt_pump`, `run_encrypt_pump`).
**Roadmap gate**: `cargo xtask lima run -- cargo xtask mutants --diff
origin/main --features integration-tests --package overdrive-dataplane
--file crates/overdrive-dataplane/src/mtls/splice.rs`; kill-rate ≥ 80%
with THREE named kill targets:

1. **delete the `shutdown(SHUT_WR)` forward** → killed by the pump-level
   tests (the dst peer never sees EOF);
2. **drop the `!state.stop` guard** → killed by
   `deliberate_teardown_does_not_forward_half_close`;
3. **flip `Graceful → TransportDeath`** (the `exit == Graceful` arm) →
   killed by the retained `graceful_eof_exit_does_not_fire_self_teardown`
   (and `transport_death_does_not_forward_half_close`).

Plus the review-03-01 **BLOCKER-2 call-site protection**: deletion of the
`forward_half_close_if_source_eof(...)` call in `run_decrypt_pump` AND in
`run_encrypt_pump` must be killed by the new pump-level tests.

**Code under test SHA**: the review-03-01 remediation of `78731515` — the
step 03-01 A1 forward (`937779d2`) plus this remediation's ADD-ONLY test +
docstring changes (no production behavior change: the
`forward_half_close_if_source_eof` logic, `mark_exited`, and both pump
loops are byte-identical to `937779d2`).
**Closes**: review-03-01 BLOCKER-1 ("the mandatory 03-01 mutation gate has
no reviewable evidence") and reinforces BLOCKER-2 ("unit tests exercise the
helper directly, not the production pump terminal path").

---

## TL;DR

- **The diff-scoped gate is SIGNALLING and PASSES at 100%.** cargo-mutants
  v27.1.0 synthesised **5 mutants over `splice.rs`, all 5 caught, 0 missed,
  0 timeout, 0 unviable → kill_rate = 100.0% → PASS**. Unlike the CLI steps
  (`mutants-01-04.md`), this step's surface is real branching logic (a guard
  `if exit != Graceful || state.stop { return; }` plus a `libc::shutdown`
  effect + two call sites), so the tool lands mutable-operator mutants on it
  — no `total=0` vacuous pass here.
- **All THREE roadmap named kill targets are discharged by the tool** (§ 1):
  the forward-deletion, the `||`→`&&` guard flip (the `!state.stop` guard),
  and the `!=`→`==` arm flip (the `Graceful`/`TransportDeath` discrimination)
  are each a distinct caught mutant.
- **BLOCKER-2 call-site protection is discharged BY THE TOOL** (§ 1): v27 DID
  emit the two call-site body-replacement mutants (`replace run_decrypt_pump
  with ()`, `replace run_encrypt_pump with ()`), and BOTH are **caught by the
  new pump-level tests** — the exact wiring the review said the old
  helper-only tests could not pin.
- **BLOCKER-2 is ALSO discharged by EXECUTED manual mutation proofs** (§ 2):
  each `forward_half_close_if_source_eof(...)` call was deleted in turn, the
  corresponding pump-level test run under Lima, the real RED pasted, and the
  source reverted via Edit (verified byte-identical). This is the honest
  belt-and-suspenders proof requested even had the tool been blind to bare
  call-site statements (per project memory
  `reference_cargo_mutants_blind_to_spawn_blocking_and_saturating_add`).
- **No mutation was ever committed.** Every hand mutation was applied with
  Edit, captured RED, and reverted before the next step.

---

## Step 0 — baseline GREEN (unmutated)

The full `mtls::splice` module passes UNMUTATED at the remediation SHA,
proving every RED below is caused by the mutation, not a pre-existing
failure. cargo-mutants confirms the same via `Unmutated baseline … ok`:

```
cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --lib \
  --features integration-tests -E 'test(mtls::splice)'
...
    Starting 10 tests across 1 binary (45 tests skipped)
     Summary [   0.516s] 10 tests run: 10 passed, 45 skipped
```

The 10 module tests: the 4 `mark_exited` boundary tests
(`transport_death_exit_fires_self_teardown_once_and_marks_gone`,
`graceful_eof_exit_does_not_fire_self_teardown`,
`deliberate_teardown_does_not_refire_even_on_transport_death`,
`transport_death_without_installed_trigger_is_a_noop`), the 3 helper-level
forward tests (`source_clean_close_forwards_half_close_to_dst_and_does_not_reclaim`,
`deliberate_teardown_does_not_forward_half_close`,
`transport_death_does_not_forward_half_close`), and the 3 NEW remediation
tests (`decrypt_pump_forwards_half_close_on_source_eof`,
`encrypt_pump_forwards_half_close_on_source_eof`,
`graceful_forward_on_dst_eof_is_harmless`).

---

## 1. The diff-scoped tool run + verbatim result (signalling PASS, 100%)

The diff-scoped run goes through Lima (`--features integration-tests`
requires it) and was backgrounded to completion per
`.claude/rules/testing.md`. The machine-parsed summary was read from the
GUEST target dir per the Lima mutation-summary-host-path trap.

```
cargo xtask lima run -- cargo xtask mutants --diff origin/main \
  --features integration-tests --package overdrive-dataplane \
  --file crates/overdrive-dataplane/src/mtls/splice.rs
```

Verbatim run tail:

```
Found 5 mutants to test
ok      Unmutated baseline in 10s build + 92s test
 INFO Auto-set test timeout to 464s
5 mutants tested in 2m: 5 caught
mutants: mode=diff total=5 caught=5 missed=0 timeout=0 unviable=0 kill_rate=100.0%
mutants: PASS
```

Guest-side `mutants-summary.json` (the structured gate record):

```json
{
  "mode": "diff",
  "cargo_mutants_version": "27.1.0",
  "total_mutants": 5,
  "caught": 5,
  "missed": 0,
  "timeout": 0,
  "unviable": 0,
  "baseline_success": 0,
  "kill_rate_pct": 100.0,
  "base_ref": "origin/main",
  "status": "pass"
}
```

Guest-side `caught.txt` — every synthesised mutant, with the named kill
target / BLOCKER it discharges (line numbers are relative to the
diff-materialised source, whose offsets shift with the docstring additions;
the item names are stable):

| Mutant | Discharges | Killed by |
|---|---|---|
| `replace forward_half_close_if_source_eof with ()` | **Named target #1** — deletes the `shutdown(SHUT_WR)` forward | pump-level `decrypt_pump…` + `encrypt_pump…` (dst peer never sees EOF) and helper-level `source_clean_close…` |
| `replace \|\| with && in forward_half_close_if_source_eof` | **Named target #2** — the guard `exit != Graceful \|\| state.stop`; `\|\|`→`&&` makes the guard require BOTH, so a `stop == true` teardown no longer suppresses the forward | `deliberate_teardown_does_not_forward_half_close` (`stop == true` ⇒ dst peer would wrongly see EOF) |
| `replace != with == in forward_half_close_if_source_eof` | **Named target #3** — the `exit != Graceful` arm; `!=`→`==` inverts the Graceful/TransportDeath discrimination | `transport_death_does_not_forward_half_close` + `source_clean_close…` (a `TransportDeath` would forward, a `Graceful` would not) |
| `replace run_decrypt_pump with ()` | **BLOCKER-2** — the decrypt call site's whole body (incl. the `forward_half_close_if_source_eof(dst_fd, exit, state)` call) | `decrypt_pump_forwards_half_close_on_source_eof` (real pump terminal path) |
| `replace run_encrypt_pump with ()` | **BLOCKER-2** — the encrypt call site's whole body (incl. the forward call) | `encrypt_pump_forwards_half_close_on_source_eof` (real pump terminal path) |

`missed.txt` is empty. **The tool's own signal already discharges all three
named targets AND the BLOCKER-2 call-site protection** — the two
`replace run_*_pump with ()` mutants are body-replacement mutants that erase
the forward call along with the loop, and the new pump-level tests are what
catch them (the old helper-only tests, which never enter the pump, could
not). The § 2 manual proofs pin the narrower "delete ONLY the call
statement" mutation the tool does not emit as a standalone mutant.

---

## 2. EXECUTED manual mutation proofs — the BLOCKER-2 call-site heart

For each proof the discriminating pump-level test was run with:

```
cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --lib \
  --features integration-tests \
  -E 'test(<decrypt|encrypt>_pump_forwards_half_close_on_source_eof)'
```

Each proof deletes ONLY the `forward_half_close_if_source_eof(dst_fd, exit,
state)` call statement (leaving the rest of the pump intact — the narrower
mutation the tool's body-replacement mutant subsumes but does not isolate),
runs the corresponding pump-level test, pastes the RED, then reverts via Edit
and confirms `git diff` shows the call site byte-identical.

### Proof A — decrypt-pump call-site deletion (BLOCKER-2, `run_decrypt_pump`)

Mutation: delete the call at `run_decrypt_pump`'s terminal exit
(`splice.rs:447`), leaving only `mark_exited(state, exit);`. The real pump
then exits without forwarding the source FIN, so the dst peer's write side
stays open and its read never surfaces EOF. The pump-level test must go RED:

```
running 1 test
test mtls::splice::tests::decrypt_pump_forwards_half_close_on_source_eof ... FAILED

thread 'mtls::splice::tests::decrypt_pump_forwards_half_close_on_source_eof'
  panicked at crates/overdrive-dataplane/src/mtls/splice.rs:843:13:
  the pump must forward the source clean-close as shutdown(SHUT_WR) to its dst
  leg — the dst peer's read must surface EOF. It did not (read timed out),
  which means the forward_half_close_if_source_eof(...) call site was not
  exercised on the pump's terminal exit (the review-03-01 BLOCKER-2 wiring).

test result: FAILED. 0 passed; 1 failed; 54 filtered out
```

Reverted via Edit; the call site is byte-identical to `937779d2`.

### Proof B — encrypt-pump call-site deletion (BLOCKER-2, `run_encrypt_pump`)

Mutation: delete the call at `run_encrypt_pump`'s terminal exit
(`splice.rs:537`), leaving only `mark_exited(state, exit);`. This is the
call site the Tier-3 S-DBN-CHURN oracle does NOT exercise (it drives the
return-decrypt path), so this pump-level test is the sole guard. It must go
RED:

```
running 1 test
test mtls::splice::tests::encrypt_pump_forwards_half_close_on_source_eof ... FAILED

thread 'mtls::splice::tests::encrypt_pump_forwards_half_close_on_source_eof'
  panicked at crates/overdrive-dataplane/src/mtls/splice.rs:843:13:
  the pump must forward the source clean-close as shutdown(SHUT_WR) to its dst
  leg — the dst peer's read must surface EOF. It did not (read timed out),
  which means the forward_half_close_if_source_eof(...) call site was not
  exercised on the pump's terminal exit (the review-03-01 BLOCKER-2 wiring).

test result: FAILED. 0 passed; 1 failed; 54 filtered out
```

Reverted via Edit; the call site is byte-identical to `937779d2`.

### Post-proof verification

After both proofs, `git diff -- crates/overdrive-dataplane/src/mtls/splice.rs`
contains NO production-call-site diff lines (a grep for `forward_half_close_if
_source_eof(dst_fd, exit, state)` add/remove lines and for the transient
`MANUAL-MUTATION-PROOF` marker returns nothing) — both production functions
are byte-identical to `937779d2`. No mutation was committed. The only
`splice.rs` diff vs HEAD is ADD-ONLY: the three new `#[test]` fns + the
`forward_half_close_if_source_eof` docstring precision (HIGH-1).

---

## 3. HIGH-1 — the non-source `Graceful` (dst-EOF) path, documented + regressed

Review HIGH-1 asked to test-and-document the non-source `Graceful` path. The
ADR-0070 amendment (§ "The teardown-vs-source-EOF distinction", lines
~727–745) DELIBERATELY groups the decrypt pump's `splice_pipe_to_dst`
`n_out == 0` DESTINATION clean-EOF as a forward case; on it,
`shutdown(dst_fd, SHUT_WR)` targets an ALREADY-CLOSED dst leg — a **deliberate
harmless no-op**. The remediation does NOT change this pinned behavior; it
adds precision + a regression test only:

- **Docstring** (`forward_half_close_if_source_eof`): now states that
  `PumpExit::Graceful` with `!state.stop` arrives from TWO shapes — a source
  clean EOF AND the `splice_pipe_to_dst` `n_out == 0` dst-EOF — and that the
  latter's `SHUT_WR` onto an already-closed dst is a deliberate harmless
  no-op. The `!state.stop` guard is named as the sole discriminator (a
  deliberate teardown is the only `Graceful` shape that does not forward).
- **Regression test** `graceful_forward_on_dst_eof_is_harmless`: drives the
  forward against a dst leg whose peer is ALREADY closed (the end-state the
  `n_out == 0` dst-EOF forward reaches) and proves the three harmlessness
  clauses — (1) the forward neither panics nor errors, (2) it does NOT fire
  (B) self-teardown on `Graceful`, (3) it touches ONLY the dst leg's write
  side (an independent sibling socketpair still round-trips a byte-distinct
  message end to end).

**Investigation note (no defect found).** An initial attempt drove the
`n_out == 0` shape through the REAL pump by fully closing the dst peer and
letting `splice_pipe_to_dst` observe the close. That produced a
`TransportDeath` exit (self-teardown fired), NOT `Graceful` — because on a
stream socket a `splice(pipe → dst)` whose read-peer has fully closed returns
`EPIPE` (correctly classified `TransportDeath`), not `0`. This is NOT a
production defect: the `n_out == 0` Graceful shape is the narrow case of a
dst peer that FIN-closed its READ side cleanly while the write returns 0, and
`EPIPE` (peer fully gone) is correctly a transport death. The production
classification in `splice_pipe_to_dst` is right. The "harmless no-op" the ADR
pins is a property of the FORWARD against an already-closed dst leg, so the
regression test pins exactly that — the honest, design-faithful proof of the
same contract, matching the docstring. No behavior changed; nothing was
surfaced as a blocker because there is no defect.

---

## 4. Conclusion — every mandatory target accounted for

| # | Mandatory target | Killed by (test) | Evidence |
|---|---|---|---|
| 1 | **delete the `shutdown(SHUT_WR)` forward** | `decrypt_pump…` + `encrypt_pump…` (pump-level) + `source_clean_close…` (helper) | Tool: `replace forward_half_close_if_source_eof with ()` caught (§ 1) |
| 2 | **drop the `!state.stop` guard** | `deliberate_teardown_does_not_forward_half_close` | Tool: `replace \|\| with && …` caught (§ 1) |
| 3 | **flip `Graceful → TransportDeath`** | `graceful_eof_exit_does_not_fire_self_teardown` + `transport_death_does_not_forward_half_close` | Tool: `replace != with == …` caught (§ 1) |
| B2 | **delete `run_decrypt_pump` call site** | `decrypt_pump_forwards_half_close_on_source_eof` | Tool: `replace run_decrypt_pump with ()` caught (§ 1) + manual Proof A RED (§ 2) |
| B2 | **delete `run_encrypt_pump` call site** | `encrypt_pump_forwards_half_close_on_source_eof` | Tool: `replace run_encrypt_pump with ()` caught (§ 1) + manual Proof B RED (§ 2) |

The diff-scoped tool gate is **signalling and passes at 100%** (5/5 caught,
0 missed) — every named target and the BLOCKER-2 call-site protection are
each a distinct caught mutant. The § 2 executed manual proofs reinforce the
BLOCKER-2 wiring by isolating the narrower call-statement deletion. HIGH-1 is
closed by the docstring precision + the `graceful_forward_on_dst_eof_is_harmless`
regression test, with the investigation confirming no production defect on
the dst-EOF path.
