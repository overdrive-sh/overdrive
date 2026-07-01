# Mutation evidence — step 01-04 (`overdrive workload restart` CLI verb)

**Step**: `01-04` — the e2e production-loop closer: the `workload restart`
CLI verb (`commands::workload::restart` + `RestartArgs` / `RestartOutput`)
and the `ApiClient::restart_workload` route binding that drives
`POST /v1/jobs/{id}/restart` into the real in-process `LocalIntentStore`.
**Surface under gate**:
`crates/overdrive-cli/src/commands/workload.rs` (the `restart` fn) and
`crates/overdrive-cli/src/http_client.rs` (the `restart_workload` route
method).
**Roadmap gate**: kill-rate ≥ 80% (`roadmap.json` step `01-04`, final
criterion) with TWO named kill targets:

1. the **CLI handler decision + `RestartOutput` shape** — the handler
   PRESERVES the server's `outcome` label (`outcome: resp.outcome`)
   rather than fabricating it, and echoes the route the POST was issued
   to, and
2. the **404 → non-zero-exit mapping** — an unknown id surfaces the
   typed `CliError::HttpStatus { status: 404, body.error == "not_found" }`
   → a non-zero exit code, never a swallowed 404 / silent `Ok` / exit 0.

**Code under test SHA**: `f2646da3` (the 01-04 CLI restart verb after
review-01-04 BLOCKER-1 strengthened the ATs — deterministic `Restarted`
outcome on the success path + the new `Resumed` scenario).
**Closes**: review-01-04 BLOCKER-2 ("the step 01-04 mutation gate is not
evidenced").

---

## TL;DR

- **Diff-scoped gate (the CI-enforced scope, `--diff origin/main`) is
  non-signalling for these two files: `total=0`, vacuous pass.** Over
  `commands/workload.rs` + `http_client.rs` cargo-mutants 27 logs
  `INFO No mutants to filter` and produces no report — the documented
  v27 blind spot for thin `async` `post_typed` delegations
  (`restart_workload` is a one-line `self.post_typed(...).await`) and
  struct-literal returns (`RestartOutput { outcome: resp.outcome, .. }`),
  the same class recorded in project memory
  `reference_cargo_mutants_blind_to_spawn_blocking_and_saturating_add`.
  The xtask wrapper correctly treats the empty filter intersection as a
  `total=0` vacuous pass — it is **not** a real kill-rate, and is
  recorded honestly in § 1.
- **Both named kill targets are discharged by EXECUTED manual mutation
  proofs** (§ 2): each hand mutation was applied with Edit, the
  discriminating acceptance test(s) run under Lima
  (`-E 'test(workload_restart)'`), the real RED output pasted, and the
  source reverted to `f2646da3` (empty diff) before the next proof. No
  mutation was ever committed.
  - **Proof A — outcome-label preservation** (two flips): hardcode
    `outcome: Resumed` ⇒ **SUCCESS AT RED** (`left: Resumed, right:
    Restarted`); hardcode `outcome: Restarted` ⇒ **RESUMED AT RED**
    (`left: Restarted, right: Resumed`). Both hardcode directions
    killed ⇒ genuine preservation (the gap BLOCKER-1 closed).
  - **Proof B — route literal**: `v1/jobs/{id}/restart` →
    `…/restart-WRONG` ⇒ **all 3 ATs RED** (the two success-path ATs
    hit `HttpStatus { 404 }`; the unknown AT's typed-`not_found`
    assertion also fails on the untyped axum default-404 — proving the
    route literal must be exactly right).
  - **Proof C — 404 → non-zero-exit**: swallow the `?` failure into a
    fabricated `Ok(RestartOutput { outcome: Restarted, .. })` ⇒
    **UNKNOWN AT RED** (`expect_err` got `Ok(RestartOutput { .. })`).

---

## Step 0 — baseline GREEN (unmutated)

The three ATs pass UNMUTATED at `f2646da3`, proving every RED below is
caused by the mutation, not a pre-existing failure:

```
cargo xtask lima run -- cargo nextest run -p overdrive-cli \
  --features integration-tests -E 'test(workload_restart)'
...
    Starting 3 tests across 5 binaries (168 tests skipped)
     Summary [   0.167s] 3 tests run: 3 passed, 168 skipped
```

---

## 1. The recorded diff-scoped tool run + verbatim result (non-signalling)

The diff-scoped run goes through Lima (`--features integration-tests`
requires it) and was run to completion (backgrounded, exit 0):

```
cargo xtask lima run -- cargo xtask mutants --diff origin/main \
  --features integration-tests --package overdrive-cli \
  --file crates/overdrive-cli/src/commands/workload.rs \
  --file crates/overdrive-cli/src/http_client.rs
```

Verbatim (captured at HEAD `f2646da3`):

```
xtask mutants: wrote …/xtask/mutants.diff (726942 bytes) for --in-diff
xtask mutants: running … cargo mutants --output … --test-tool=nextest \
  --in-diff …/mutants.diff --file crates/overdrive-cli/src/commands/workload.rs \
  --file crates/overdrive-cli/src/http_client.rs --package overdrive-cli \
  --test-workspace=false --features integration-tests (NEXTEST_PROFILE=mutants)
 INFO No mutants to filter
xtask mutants: cargo-mutants produced no report (filter intersection is empty) — treating as zero-mutant vacuous pass
mutants: mode=diff total=0 — no mutants in scope (filter intersection empty); vacuous pass
mutants: PASS
```

The diff lines this step touches are: (a) `http_client.rs::restart_workload`
— a single thin `async` delegation `self.post_typed(&format!("v1/jobs/{id}/restart"),
&serde_json::json!({})).await`; and (b) `commands/workload.rs::restart` —
client-side validation, a config load, the `restart_workload` call, and a
struct-literal `RestartOutput { workload_id: resp.workload_id, outcome:
resp.outcome, endpoint }` return. cargo-mutants v27 synthesises **no
mutable-operator mutant** that overlaps these diff lines: the thin
`async fn` body and the struct-literal field projections fall into the
documented v27 blind spot — there is no comparison operator to flip, no
`Default`-returnable arm to erase, no boolean to negate that the tool's
`--in-diff` synthesiser lands on. The wrapper reports `INFO No mutants to
filter` → `total=0` and treats it as a **vacuous pass** (kill rate
undefined). This is **not** a real signal — recorded honestly here so the
executed manual proofs in § 2 are the substantive evidence, exactly as
`mutants-01-03.md` did for its tool-blind (`unviable`) target.

The optional whole-file `--workspace --package --file` scope was
**skipped** for the same reason `mutants-01-03.md` skipped it:
`http_client.rs` carries many other endpoint methods (`submit_workload`,
`stop_workload`, `describe_workload`, `cluster_status`, …) and a whole-file
run would mutate all of them at multi-minute cost without adding signal for
*this* step's two named targets. The substantive evidence the review
requires is the executed manual proofs below, each targeting exactly one
named kill target.

---

## 2. EXECUTED manual mutation proofs (the substantive evidence)

For each proof the discriminating ATs were run with:

```
cargo xtask lima run -- cargo nextest run -p overdrive-cli \
  --features integration-tests -E 'test(workload_restart)'
```

(That filter runs the 3 ATs in `tests/integration/workload_restart.rs`:
`…_for_declared_workload_returns_restart_output` (SUCCESS),
`…_of_stopped_workload_returns_resumed` (RESUMED),
`…_for_unknown_workload_returns_typed_404_and_nonzero_exit` (UNKNOWN).)
After each proof the source was reverted with Edit and `git diff -- <file>`
confirmed empty.

### Proof A — CLI handler decision + `RestartOutput.outcome` preservation (kill target #1)

Two flips on `commands/workload.rs` — the `restart` fn's
`RestartOutput { … outcome: resp.outcome … }` construction. Each hardcodes
one outcome literal, severing the server-label preservation:

**Flip A.1** — `outcome: resp.outcome` → `outcome: RestartOutcome::Resumed`.
The fixture deploys `payments` with NO `/stop` sentinel ⇒ the server
returns `Restarted`; a hardcoded `Resumed` diverges. The SUCCESS AT must
go RED:

```
FAIL (2/3) integration::workload_restart::workload_restart_for_declared_workload_returns_restart_output
  panicked at …/workload_restart.rs:144:5:
  assertion `left == right` failed: absent /stop sentinel ⇒ the CLI must
  preserve the server's Restarted label
    left: Resumed
   right: Restarted

Summary: 3 tests run: 2 passed, 1 failed
```

Reverted; `git diff -- commands/workload.rs` empty.

**Flip A.2** — `outcome: resp.outcome` → `outcome: RestartOutcome::Restarted`.
The RESUMED fixture stops `payments` through the production stop verb
(writing `/stop`) ⇒ the server returns `Resumed`; a hardcoded `Restarted`
diverges. The RESUMED AT must go RED:

```
FAIL (3/3) integration::workload_restart::workload_restart_of_stopped_workload_returns_resumed
  panicked at …/workload_restart.rs:209:5:
  assertion `left == right` failed: present /stop sentinel ⇒ the CLI must
  preserve the server's Resumed label
    left: Restarted
   right: Resumed

Summary: 3 tests run: 2 passed, 1 failed
```

Reverted; `git diff -- commands/workload.rs` empty.

**Both hardcode directions are killed** — `Resumed`-hardcode by the
SUCCESS AT, `Restarted`-hardcode by the RESUMED AT. Neither literal can
survive, so the handler genuinely PRESERVES the server's label
(`outcome: resp.outcome`) rather than fabricating it. This is exactly the
preservation gap review-01-04 BLOCKER-1 closed (the single deterministic
SUCCESS AT alone could not kill the `Restarted`-hardcode; the new RESUMED
scenario is what pins the second direction).

### Proof B — route literal (handler decision, kill target #1)

Mutation: `http_client.rs::restart_workload` route literal

```rust
self.post_typed(&format!("v1/jobs/{id}/restart"), &serde_json::json!({})).await
// ← mutated to "v1/jobs/{id}/restart-WRONG"
```

Under the mutation the POST hits an unrouted path; axum returns its
default 404 (no typed `ErrorBody`). Result: **all 3 ATs FAILED** — the
mutation is decisively killed:

```
FAIL (1/3) integration::workload_restart::workload_restart_for_unknown_workload_returns_typed_404_and_nonzero_exit
  panicked at …/workload_restart.rs:238:13:
  assertion `left == right` failed: error class must be `not_found`
    left: "unknown"
   right: "not_found"

FAIL (2/3) integration::workload_restart::workload_restart_for_declared_workload_returns_restart_output
  panicked at …/workload_restart.rs:133:6:
  workload::restart must succeed for a declared workload: HttpStatus { status: 404,
  body: ErrorBody { error: "unknown", message: "control plane returned HTTP 404
  Not Found with no typed body", field: None } }

FAIL (3/3) integration::workload_restart::workload_restart_of_stopped_workload_returns_resumed
  panicked at …/workload_restart.rs:203:6:
  restart of a stopped workload must succeed: HttpStatus { status: 404,
  body: ErrorBody { error: "unknown", message: "control plane returned HTTP 404
  Not Found with no typed body", field: None } }

Summary: 3 tests run: 0 passed, 3 failed
```

The two success-path ATs (SUCCESS, RESUMED) hit `HttpStatus { 404 }` — the
declared `payments` is unreachable on the wrong route. The UNKNOWN AT goes
RED too, on its typed-class assertion (`error: "unknown"` vs `not_found`):
the `restart-WRONG` route returns axum's *untyped* default 404, so the
route literal must be exactly `…/restart` for even the unknown-id path to
surface the *typed* `not_found` body the handler emits. Reverted;
`git diff -- http_client.rs` empty. (This is exactly the
`restart-WRONG` mutation an interrupted run had left dangling; re-applied
cleanly, captured, reverted.)

### Proof C — 404 → non-zero-exit mapping (kill target #2)

Mutation: `commands/workload.rs::restart` — swallow the `?` propagation
into a fabricated success, so an unknown id no longer surfaces the typed
404 Err:

```rust
let resp = match client.restart_workload(&args.id).await {
    Ok(resp) => resp,
    Err(_) => {
        return Ok(RestartOutput {
            workload_id: args.id.clone(),
            outcome: RestartOutcome::Restarted,
            endpoint,
        });
    }
};
```

Under the mutation the UNKNOWN AT's `expect_err` gets an `Ok` instead.
Result: **UNKNOWN AT FAILED** — the mutation is decisively killed:

```
FAIL (1/3) integration::workload_restart::workload_restart_for_unknown_workload_returns_typed_404_and_nonzero_exit
  panicked at …/workload_restart.rs:233:6:
  workload::restart must fail for an undeclared workload: RestartOutput {
  workload_id: "nonexistent", outcome: Restarted, endpoint: Url { … host:
  Some(Ipv4(127.0.0.1)), port: Some(37995), path: "/", … } }

Summary: 3 tests run: 2 passed, 1 failed
```

The swallowed 404 surfaces as a fabricated `Ok(RestartOutput { … outcome:
Restarted })` — exactly the silent-success / exit-0 failure the kill
target names. The UNKNOWN AT's `expect_err` (and the downstream
`cli_error_to_exit_code(&err)` non-zero assertion that depends on the Err
ever being returned) is what kills it. The other 2 ATs pass — they restart
a real workload, so the `Ok` branch is taken normally and the swallow path
is never reached. Reverted; `git diff -- commands/workload.rs` empty.

### Post-proof verification

After all four proofs (Proof A's two flips + Proof B + Proof C),
`git diff -- crates/overdrive-cli/src/commands/workload.rs
crates/overdrive-cli/src/http_client.rs` is **empty** — both production
files are byte-identical to `f2646da3`. No mutation was committed.

---

## 3. Conclusion — every mandatory target accounted for

| # | Mandatory kill target | Killed by (AT) | Evidence |
|---|---|---|---|
| 1 | **CLI handler decision + `RestartOutput` shape** — handler PRESERVES the server's `outcome` (not hardcoded), drives the correct route | SUCCESS (`…_returns_restart_output`) + RESUMED (`…_returns_resumed`) | **Proof A** — hardcode `Resumed` ⇒ SUCCESS RED `left: Resumed, right: Restarted`; hardcode `Restarted` ⇒ RESUMED RED `left: Restarted, right: Resumed` (both directions). **Proof B** — `…/restart`→`…/restart-WRONG` ⇒ all 3 ATs RED (404 / untyped-body) (§ 2) |
| 2 | **404 → non-zero-exit** — a mutation that swallows the 404 / exits 0 on an unknown id | UNKNOWN (`…_returns_typed_404_and_nonzero_exit`) | **Proof C** — swallow `?` into `Ok(RestartOutput { outcome: Restarted, .. })` ⇒ UNKNOWN RED (`expect_err` got `Ok(RestartOutput { … })`) (§ 2) |

The diff-scoped tool gate is **non-signalling** for these two files: it
produces `total=0` — `INFO No mutants to filter`, the documented
cargo-mutants v27 blind spot for thin `async post_typed` delegations and
struct-literal field projections (project memory
`reference_cargo_mutants_blind_to_spawn_blocking_and_saturating_add`) — and
the wrapper correctly records it as a **vacuous pass**, not a phantom
kill-rate (§ 1). The four **executed** manual proofs in § 2 discharge both
named kill targets and ARE the substantive evidence: the
outcome-preservation pair (Proof A, the gap BLOCKER-1 closed), the route
binding (Proof B), and the 404 → non-zero-exit honesty (Proof C).
