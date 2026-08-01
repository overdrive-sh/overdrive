# Litmus and execution evidence — `mtls-intercept-install-fault-seam` (GH #250)

The durable record of the falsification evidence each DELIVER step was gated on.
`execution-log.json` carries the phase ledger (`{sid, p, s, d, t}`); its schema
cannot hold prose, so the observations the roadmap demanded — *"record the
observed failing test name and assertion on the step"* — live here.

Governing rule: `distill/red-classification.md` § 5 — **a green suite with no
litmus recorded is an UNFALSIFIED PASS and must be rejected.** This file is what
makes that gate checkable after the session ends.

Every revert used the `Edit` tool. `git checkout --` is blocked by the
destructive-git-ops hook and was never used.

---

## Step 01-01 — `ce44a5e3`

Tests authored GREEN (the production code already existed), so the RED evidence
is the mutation-edit litmus, inverted in time.

### L-1 — whole body → `Ok(())`

- **Edit**: `action_shim/mod.rs`, `return Ok(());` inserted at the head of
  `fail_closed_on_mtls_install`'s body, rendering the whole body unreachable.
- **RED** (`--no-fail-fast`, `3 tests run: 0 passed, 3 failed, 196 skipped`):

| Test | Message |
|---|---|
| `install_failure_supersedes_running_with_failed_and_never_releases_the_gate` | `assertion 'left == right' failed: [outbound nft-TPROXY install] the fail-closed handler must supersede Running with Failed` — `left: Running`, `right: Failed` |
| `a_vanished_workload_still_yields_the_superseding_failed_row` | same shape — `left: Running`, `right: Failed` |
| `a_rejected_observation_write_surfaces_and_announces_nothing` | `a rejected observation write must be REPORTED as ShimError::Observation, never swallowed; got Ok(())` |

  All three are plain assertion failures, not scaffold panics.

**L-1 is independently corroborated mechanically**: cargo-mutants generates the
same `FnValue → 'Ok(())'` replacement and reports it `CaughtMutant` — a stronger
form of the same transformation, applied by the tool against an independently
built binary.

### L-2 — best-effort stop

- **Edit**: `mod.rs:417`, `let _ = driver.stop(handle).await;` → `driver.stop(handle).await?;`
- **RED**: `a_vanished_workload_still_yields_the_superseding_failed_row` only
  (`3 tests run: 2 passed, 1 failed`) — `a NotFound stop is tolerated — the
  handler must still report Ok(()); got Err(Driver(NotFound { alloc:
  AllocationId("mif-alloc-0") }))`

### L-3 — write-then-emit ordering

- **Edit**: `mod.rs:446`, `obs.write(..).await?;` → `let _ = obs.write(..).await;`
- **RED**: `a_rejected_observation_write_surfaces_and_announces_nothing` only
  (`3 tests run: 2 passed, 1 failed`) — `a rejected observation write must be
  REPORTED as ShimError::Observation, never swallowed; got Ok(())`

### Structural proof no revert drifted

After all litmus applications and reverts,
`git diff -U0 -- crates/overdrive-control-plane/src/action_shim/mod.rs | grep '^-'`
returned **exactly the 10 lines of the `// mutants: skip` block and nothing
else** — so the helper's body came back byte-identical each time.

### Mutation

1 mutant in `fail_closed_on_mtls_install`, `CaughtMutant`. Reported as
**VACUOUS** at the time: a 100% rate over a one-mutant set is not a coverage
claim. (The diff-scoped `--file` shape yields `No mutants to filter` because
01-01's diff is entirely `#[cfg(test)]`; the whole-file
`--workspace --package --file` shape is required.)

---

## Step 03-01 — `1fe5ed76`

The prescribed mutation gate returned **`Found 0 mutants to test`** — not a
vacuous diff-shape artifact but a real exclusion: `.cargo/mutants.toml` Rule 7
excludes `crates/overdrive-sim/src/adapters/**` wholesale. **This was NOT
reported as a kill rate.** The four load-bearing mutants were closed by manual
flip-proof instead:

| # | Mutant | Killed by | Observed |
|---|---|---|---|
| 1 | `materialise` drops the armed errno (`from_raw_os_error(errno+1)`) | S-MIF-06, S-MIF-07 | `left: Some(2) right: Some(1)` |
| 2 | `armed()` uses `.take()` instead of `.clone()` (DFS-4) | S-MIF-07 | `an armed bind fault short-circuits before any syscall: TcpListener { addr: 127.0.0.1:35173, fd: 3 }` |
| 3 | `clear_faults` omits the inbound slot | S-MIF-08 | `clear_faults disarms the inbound slot: TproxyInstall { reason: "ip rule add exited 2" }` |
| 4 | `script_inbound_fault` writes `outbound_fault` (aliased slot) | S-MIF-13, S-MIF-06 | `an inbound fault does not leak into install_outbound: TproxyInstall {...}` |

4/4 killed; post-revert re-run 239/239 green.

**Mutant 2 doubles as the positive control for the I/O-free claim**: when
`.take()` made the `Ok` arm reachable on the second call, the test failed
carrying a literally-bound socket in its panic payload — direct proof the
unmutated code never reaches the bind.

---

## Step 04-01 — `2deeaa49`

### L-4 — StartAllocation arm

- **Edit**: the `if let Some(handle) = &handle_opt { driver.release_for_exit_emission(handle); }`
  block moved **above** the mTLS guard's `return`.
- **RED**: `integration::mtls_install_fail_closed::start_allocation_install_failure_never_releases_the_exit_watcher`
- **Assertion**: `S-MIF-04 A-6': a now-Failed allocation must NEVER release its
  exit watcher — the fail-closed arm must return BEFORE
  driver.release_for_exit_emission, got releases [AllocationId("mif-start")]`

### L-5 — RestartAllocation arm

- **Edit**: the same move in the Restart arm.
- **RED**: `restart_allocation_install_failure_never_releases_the_exit_watcher`
- **Assertion**: same shape — `got releases [AllocationId("mif-restart")]`

**A-6' dies on the reordering on both arms independently, and survives step
01-01's T1 entirely** — a helper-level test structurally cannot reach a
call-site ordering property. This is the port's sole justification, discharged.

### Execution proof (root, not skipped)

Both printed past-the-gate markers: `EXECUTED S-MIF-04 (root)` /
`EXECUTED S-MIF-05 (root)`. The real netns provision ran — the Running row
carries `workload_addr: Some(10.99.0.2)`, injected only by
`provision_and_inject_netns`.

### Note on the phase ledger

04-01's ledger reads `GREEN/EXECUTED/FAIL → COMMIT/SKIPPED/BLOCKED_BY_DEPENDENCY
→ COMMIT/EXECUTED/PASS`, with **no** superseding `GREEN/PASS`. That is the
honest record, not a gap: GREEN genuinely failed because A-1' could not pass
against the then-undiagnosed production defect below, and the step landed with
A-1' `#[ignore]`d against a reproduced diagnosis. Step 04-02 fixed the defect
and un-ignored it. Injecting a retroactive `GREEN/PASS` would misreport what
happened. `des-verify-integrity` accepts the sequence as complete.

---

## Step 04-02 — `e116b7c1` — the one production fix

A-1' reproduced a real production defect: `fail_closed_on_mtls_install` built
its superseding `Failed` row from the same `tick` and `node_id` as the `Running`
row, so both carried a byte-identical `LogicalTimestamp`, `dominates()` returned
`false`, and the `Failed` row was **silently dropped by both `ObservationStore`
adapters** — leaving the allocation durably recorded `Running` with no
interception installed.

### L-6 — default lane

- **Edit**: `superseding_timestamp`, `superseded.updated_at.counter.saturating_add(1)`
  → `superseded.updated_at.counter`
- **RED**: `action_shim::fail_closed_mtls_tests::install_failure_supersedes_running_with_failed_and_never_releases_the_gate`
  and `::a_vanished_workload_still_yields_the_superseding_failed_row` (all six cases)
- **Assertion**: `assertion left == right failed: [outbound nft-TPROXY install]
  the fail-closed handler must supersede Running with Failed` — `left: Running,
  right: Failed` (`mod.rs:2473`; `:2658` for the vanished-workload case)
- Stronger than predicted: the tie makes LWW drop the row *entirely*, so A-2's
  state assertion fires before the counter assertion is reached.

### L-7 — Lima + root — the litmus that proves the fix

- **Edit**: `fail_closed_on_mtls_install`'s 5th argument
  `superseding_timestamp(tick, running_row)` → `timestamp_for(tick, running_row.node_id.clone())`,
  reintroducing the exact defect.
- **RED, both arms**: `start_allocation_install_failure_supersedes_running_with_failed`
  (S-MIF-04) and `restart_allocation_install_failure_supersedes_running_with_failed` (S-MIF-05)
- **Assertion** (both): `the dispatch must write exactly two rows for the alloc
  — a Running row and the Failed row that supersedes it` — `left: 1, right: 2`,
  the surviving row being `state: Running, updated_at: LogicalTimestamp {
  counter: 1, writer: NodeId("node-001") }`

**Decisive detail**: under the reintroduced defect the *other two* tests
(A-6'/A-8'/A-9') still **passed**. That is direct proof A-1' is the only
assertion observing supersession, and that un-ignoring it was justified.

### Execution proof

`EXECUTED S-MIF-04 A-1' (root)` / `EXECUTED S-MIF-05 A-1' (root)`.

### Mutation — verbatim enumeration, then the honest finding

Guest `mutants.out`: 3 mutants, 3 caught, 0 missed, 0 unviable.

```
mod.rs:426:5: replace fail_closed_on_mtls_install -> Result<(), ShimError> with Ok(())
mod.rs:525:5: replace fail_closed_on_netns_provision -> Result<(), ShimError> with Ok(())
mod.rs:934:5: replace dispatch_single -> Result<(), ShimError> with Ok(())
```

**`superseding_timestamp` generated ZERO mutants** — `LogicalTimestamp` derives
no `Default` (whole-body operator unviable), `saturating_add(literal)` yields
nothing in this repo, and `.max(..)` is a method call, not a mutable binary
operator. **The 100% therefore says nothing whatsoever about this fix**; its
defense rests entirely on L-6 and L-7. No `exclude_re` entry was added.

---

## Step 05-01 — `65fdbeaf`

### Execution proof — all four, both adapters, root, zero skips

Kernel `7.0.0-28-generic`:

```
[S-MIF-09][host] EXECUTED — local_addr = 127.0.0.1:42627
[S-MIF-09][sim]  EXECUTED — local_addr = 127.0.0.1:41557
[S-MIF-10][host] EXECUTED — leg-F 127.0.0.1:39069, leg-C 127.0.0.1:43213 (both held live)
[S-MIF-10][sim]  EXECUTED — leg-F 127.0.0.1:38333, leg-C 127.0.0.1:38369 (both held live)
[S-MIF-11][host] EXECUTED — install_outbound(ovd-hv-eq0501, 43029) and install_inbound(10.99.5.1:18501, 45477) both Ok; both guards released cleanly
[S-MIF-11][sim]  EXECUTED — install_outbound(ovd-hv-eq0501, 38959) and install_inbound(10.99.5.1:18501, 44667) both Ok; both guards released cleanly
[S-MIF-12][host] EXECUTED — two installs of (ovd-hv-eq0501, 35991) both Ok; both guards released in turn, the second over already-released state
[S-MIF-12][sim]  EXECUTED — two installs of (ovd-hv-eq0501, 42911) both Ok; both guards released in turn, the second over already-released state
Summary [0.238s] 4 tests run: 4 passed
```

### Final unscoped per-PR mutation gate — the gate CI runs

`cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests`

```
mutants: mode=diff total=4 caught=4 missed=0 timeout=0 unviable=0 kill_rate=100.0% status=pass
```

| Outcome | Site | Replacement |
|---|---|---|
| **Caught** | `action_shim/mod.rs:426` **`fail_closed_on_mtls_install`** | `Ok(())` |
| Caught | `action_shim/mod.rs:525` `fail_closed_on_netns_provision` | `Ok(())` |
| Caught | `action_shim/mod.rs:934` `dispatch_single` | `Ok(())` |
| Caught | `mtls_intercept_worker.rs:534` `MtlsInterceptWorker::start_alloc` | `Ok(())` |

**This closes GH #250.** cargo-mutants now *generates* the
`fail_closed_on_mtls_install -> Ok(())` whole-body mutant — the `exclude_re`
deletion held, it is no longer suppressed — and it is **caught**. The gate
reported zero missed mutants, so no `exclude_re` entry was added for
`HostMtlsIntercept`'s delegations.

### Leak hygiene

Pre-sweep found real pre-existing debt (a leftover `overdrive-mtls` nft table,
two `ovd-veth-*` veths). After this suite's scoped run: zero nft table, veths,
fwmark rules, table-100 routes, netns. After the workspace + mutation runs,
leftovers from *sibling* suites (`vethH-bidi05`, `nsW-bidi0501`,
`ovd-veth-bk`/`-cli`, ~200 `alloc-coinflip-*.scope`) were swept; final verify is
`nft: table ip nat` only, zero veths, zero netns, zero alloc cgroups.

---

## Suite results at the close

| Gate | Result |
|---|---|
| `cargo nextest run --workspace --features integration-tests` | **2467 passed**, 22 skipped |
| `cargo nextest run -p overdrive-worker --features integration-tests` | 188/188 |
| `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | clean |
| `cargo fmt --all --check` | clean |
| `des-verify-integrity docs/feature/.../deliver/` | **All 6 steps have complete DES traces** |
