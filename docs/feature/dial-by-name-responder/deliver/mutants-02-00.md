# Mutation-gate evidence — step 02-00 (MtlsResolve re-key)

Resolves review finding **D3** (`review-02-00.md` § "Re-review — corrective commit 69948303"):
the mandated per-step mutation gate over
`crates/overdrive-control-plane/src/mtls_resolve_adapter.rs` had been run but left no
durable, committed evidence. This artifact pins both gate runs with real `cargo-mutants`
output and the guest-summary kill-rates.

- **Target file:** `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs`
- **Base ref:** `origin/main`
- **cargo-mutants:** v27.1.0 (Lima guest, rustup 1.95.0-aarch64-unknown-linux-gnu)
- **Killing tests:** the `mtls_resolve_rekey.rs` + `dns_name_index.rs` acceptance suites
  (REKEY-01..04, FAILCLOSED-01, the `equiv_01_classify_matches_independent_reference_oracle`
  reference-oracle property, and the COHERENCE byte-identity / fail-closed properties).

---

## Why run WITHOUT `--features integration-tests`

The roadmap criterion (#10) names `--features integration-tests`, but the killing tests for
this slice are **acceptance tests that stay in the default unit lane**.
`crates/overdrive-control-plane/tests/acceptance.rs` states verbatim: *"Acceptance tests in
this crate stay in the default unit lane — they exercise only in-process serde round-trips
and utoipa schema emission, no real infrastructure."* `mtls_resolve_rekey.rs` and
`dns_name_index.rs` — the tests that kill the re-key mutants — therefore run **without** the
`integration-tests` feature.

Consequences of dropping the feature for this gate, all favourable and all sanctioned by
`.claude/rules/testing.md`:

1. **The killing tests still participate.** They are default-lane; the feature would not add
   any test that kills a `mtls_resolve_adapter.rs` re-key mutant.
2. **Clean baseline.** Enabling `integration-tests` pulls in the `overdrive-mtls` nft-table
   integration suite, which has a known parallel-baseline race (project memory
   `reference_mutation_integration_nft_baseline_race`) that can FAIL the *unmutated* baseline
   non-deterministically. Without the feature the baseline is clean (both runs:
   `ok Unmutated baseline`).
3. **Faster** — no integration-suite compile/run per mutant.

This is the sanctioned shape for a default-lane-tested file:
`.claude/rules/testing.md` § "Mutation testing" explicitly permits dropping the feature when
deliberately measuring kill rate without integration tests participating, and here the
default lane *is* where the coverage lives.

---

## Gate 1 — diff-scoped (the roadmap criterion #10 gate)

This is the gate the roadmap mandates: `--diff origin/main`, scoped to this step's changed
lines in `mtls_resolve_adapter.rs`. **PASS at 100% (2/2), zero in-scope survivors.**

```
cargo xtask lima run -- cargo xtask mutants --diff origin/main \
  --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/mtls_resolve_adapter.rs
```

Real output (tail):

```
xtask mutants: wrote .../mutants.diff (1078353 bytes) for --in-diff
xtask mutants: running .../cargo mutants --output .../xtask --test-tool=nextest \
  --in-diff .../mutants.diff --file crates/overdrive-control-plane/src/mtls_resolve_adapter.rs \
  --package overdrive-control-plane --test-workspace=false (NEXTEST_PROFILE=mutants)
Found 2 mutants to test
ok      Unmutated baseline in 14s build + 6s test
 INFO Auto-set test timeout to 32s
2 mutants tested in 36s: 2 caught
mutants: mode=diff total=2 caught=2 missed=0 timeout=0 unviable=0 kill_rate=100.0%
mutants: PASS
```

GUEST summary (`/home/marcus.guest/.cargo-target-lima/xtask/mutants-summary.json` — the host
`target/xtask/mutants-summary.json` is stale on macOS Lima, project memory
`reference_lima_mutation_summary_host_path_trap`):

```json
{
  "mode": "diff",
  "cargo_mutants_version": "27.1.0",
  "total_mutants": 2,
  "caught": 2,
  "missed": 0,
  "timeout": 0,
  "unviable": 0,
  "baseline_success": 0,
  "kill_rate_pct": 100.0,
  "base_ref": "origin/main",
  "status": "pass"
}
```

Caught (2/2) — exactly the re-key call sites the criterion names as primary kill targets:

```
mtls_resolve_adapter.rs:395:9: replace BackendIndex::bind_frontend with ()
mtls_resolve_adapter.rs:406:9: replace BackendIndex::first_healthy_backend_for -> Option<SocketAddrV4> with None
```

Missed: none. Unviable: none.

**Verdict: diff-scoped kill_rate = 100.0% ≥ 80% → PASS, zero in-scope survivors.** The
`bind_frontend` insert path and the `first_healthy_backend_for` first-by-`Ord` selection are
both mutation-covered, proving the `equiv_01` reference oracle's claimed kill power
(a `min→max` / arm-flatten bug diverges the oracle vs production trajectories) is real.

---

## Gate 2 — whole-file (additional evidence for the classify arms)

A whole-file run (not the mandated gate) to evidence coverage of the classify-arm and
adapter call sites the 2-mutant diff run does not exercise. **90.9% (10 caught / 4 unviable /
1 missed).** The single survivor is pre-existing and out of 02-00's diff (see below).

```
cargo xtask lima run -- cargo xtask mutants --workspace \
  --package overdrive-control-plane \
  --file crates/overdrive-control-plane/src/mtls_resolve_adapter.rs
```

Real output (tail):

```
Found 15 mutants to test
ok      Unmutated baseline in 12s build + 6s test
 INFO Auto-set test timeout to 33s
MISSED  crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:659:9: replace <impl Drop for ServiceBackendsResolve>::drop with () in 3s build + 6s test
15 mutants tested in 82s: 1 missed, 10 caught, 4 unviable
mutants: mode=workspace total=15 caught=10 missed=1 timeout=0 unviable=4 kill_rate=90.9%
mutants: baseline=100.0% drift=-9.1pp
mutants: WARN — mutants drift -9.1pp below baseline 100.0% (current=90.9%)
```

GUEST summary:

```json
{
  "mode": "workspace",
  "cargo_mutants_version": "27.1.0",
  "total_mutants": 15,
  "caught": 10,
  "missed": 1,
  "timeout": 0,
  "unviable": 4,
  "baseline_success": 0,
  "kill_rate_pct": 90.9,
  "baseline_pct": 100.0,
  "drift_pp": -9.1,
  "baseline_path": "mutants-baseline/main/kill_rate.txt",
  "status": "warn",
  "reason": "mutants drift -9.1pp below baseline 100.0% (current=90.9%)"
}
```

### Caught (10)

```
mtls_resolve_adapter.rs:314:9: replace BackendIndex::apply_row with ()
mtls_resolve_adapter.rs:356:9: replace BackendIndex::replace_from_snapshot with ()
mtls_resolve_adapter.rs:395:9: replace BackendIndex::bind_frontend with ()
mtls_resolve_adapter.rs:406:9: replace BackendIndex::first_healthy_backend_for -> Option<SocketAddrV4> with None
mtls_resolve_adapter.rs:491:33: replace match guard by_service.values().any(|backend| backend.healthy) with true  in BackendIndex::classify_by_addr
mtls_resolve_adapter.rs:491:33: replace match guard by_service.values().any(|backend| backend.healthy) with false in BackendIndex::classify_by_addr
mtls_resolve_adapter.rs:563:9: replace ServiceBackendsResolve::relist -> Result<(), String> with Ok(())
mtls_resolve_adapter.rs:577:9: replace ServiceBackendsResolve::relist_into -> Result<(), String> with Ok(())
mtls_resolve_adapter.rs:678:9: replace <impl MtlsResolve for ServiceBackendsResolve>::probe -> Result<()> with Ok(())
mtls_resolve_adapter.rs:724:12: delete ! in <impl MtlsResolve for ServiceBackendsResolve>::resolve
```

Both `classify_by_addr` healthy-match-guard mutants (→true AND →false) are killed — the
arm-3 healthy/unhealthy discrimination is fully covered, alongside the re-key arms.

### Unviable (4) — all `spawn_drain` JoinHandle constructions (do not compile; not coverage gaps)

```
mtls_resolve_adapter.rs:611:9: replace ServiceBackendsResolve::spawn_drain -> JoinHandle<()> with JoinHandle::new()
mtls_resolve_adapter.rs:611:9: replace ServiceBackendsResolve::spawn_drain -> JoinHandle<()> with JoinHandle::from_iter([()])
mtls_resolve_adapter.rs:611:9: replace ServiceBackendsResolve::spawn_drain -> JoinHandle<()> with JoinHandle::new(())
mtls_resolve_adapter.rs:611:9: replace ServiceBackendsResolve::spawn_drain -> JoinHandle<()> with JoinHandle::from(())
```

`JoinHandle` has no such constructors, so these mutants fail to compile — `unviable`, not
`missed`. They are excluded from the kill-rate denominator.

### Missed (1) — PRE-EXISTING, out of 02-00's diff

```
mtls_resolve_adapter.rs:659:9: replace <impl Drop for ServiceBackendsResolve>::drop with ()
```

This is the abort-on-drop guard of the single-owner drain task (best-effort, fire-and-forget
cleanup). It is:

1. **Not in 02-00's diff vs `origin/main`.** Verified:
   `git diff origin/main -- crates/overdrive-control-plane/src/mtls_resolve_adapter.rs | grep -E 'impl Drop|fn drop'`
   returns nothing — no `Drop`-impl change landed in 02-00. The diff-scoped gate (the actual
   roadmap gate) consequently does not include this mutant, and is 100%.
2. **Genuinely hard to unit-test.** The production docstring at `:650-654` already records
   why: *"there is no synchronous, in-process observable to assert on through the public
   surface (Drop cannot await the abort), so a mutant that empties this body is behaviourally
   indistinguishable in a test."* Its sole symptom is a "still-running task at teardown"
   nextest leak report — not a public-surface observable.
3. **Out of 02-00's scope.** Per the dispatch, no test is added for it; it is documented here
   as a pre-existing, out-of-scope survivor.

---

## Summary

| Gate | Mode | Total | Caught | Unviable | Missed | Kill-rate | Verdict |
|---|---|---|---|---|---|---|---|
| **Roadmap criterion #10** | diff vs origin/main | 2 | 2 | 0 | 0 | **100.0%** | **PASS** (≥80%, zero in-scope survivors) |
| Whole-file (informational) | workspace | 15 | 10 | 4 | 1 | 90.9% | WARN (drift vs 100% baseline; the 1 survivor is pre-existing + out-of-diff) |

The **mandated per-step diff-scoped gate is 100% with zero in-scope survivors** — well above
the ≥80% threshold. The re-key call sites (`bind_frontend`, `first_healthy_backend_for`
first-by-`Ord`), the three-way `classify_by_addr` healthy-arm discrimination, the per-service
`apply_row` eviction, and the relist/probe/resolve adapter paths are all mutation-covered.
The single whole-file survivor (`Drop::drop` at `:659`) is pre-existing, out of 02-00's diff,
genuinely hard to unit-test, and explicitly out of this step's scope — no test added for it.

---

## MUTATION-phase log status — CLI rejected the phase name

`des-log-phase` does NOT accept a `MUTATION` phase. The invocation

```
des-log-phase --project-dir docs/feature/dial-by-name-responder/deliver \
  --step-id 02-00 --phase MUTATION --status EXECUTED --data "..."
```

was rejected with:

```
Error: Invalid phase 'MUTATION'. Valid phases: PREPARE, RED_ACCEPTANCE, RED_UNIT, GREEN, COMMIT
```

Per the dispatch ("If the CLI rejects `MUTATION`, record the same in the evidence artifact and
report the rejection — do not invent a different phase"), the mutation-gate result is recorded
HERE instead of in `execution-log.json`. The authoritative kill-rate record for 02-00 is this
artifact: **diff-scoped (roadmap criterion #10) = 100.0% (2/2 caught), PASS, zero in-scope
survivors; whole-file = 90.9% (10 caught / 4 unviable / 1 pre-existing-out-of-diff missed)**.
No alternate phase was substituted.
