# RED-classification PLAN — `backend-instance-replacement`

**Wave**: DISTILL (the PLAN) → DELIVER's RED phase (the actual run).
**Designer**: Quinn | **Date**: 2026-06-30 | **Feature**: GH #249

Per ADR-025 D2, the pre-DELIVER **fail-for-the-right-reason gate** becomes
DELIVER's RED-phase entry/exit gate. DISTILL authors the scenarios
(`test-scenarios.md`) and this PLAN; **DISTILL does NOT run the classification**
— the production surface this feature builds (the `TxnOp::IncrementU64` variant,
the `restart_workload` handler + route, the `overdrive workload restart` CLI
verb, the `WorkloadLifecycle` generation fields + `current_alloc` helper) does
not exist yet (see § "Why the classification runs in DELIVER, not DISTILL").
DELIVER's RED phase materialises the scaffolds (per the Scaffold MANIFEST in
`feature-delta.md` § "Wave: DISTILL / [REF] Scaffold MANIFEST") and runs the gate.

## Expected RED failure mode for every SCAFFOLDED test: `MISSING_FUNCTIONALITY`

Every NEW scenario, once scaffolded, MUST fail with `MISSING_FUNCTIONALITY` —
the production behaviour is unimplemented — NOT `IMPORT_ERROR` /
`FIXTURE_BROKEN` / `SETUP_FAILURE` / `WRONG_ASSERTION`.

In Rust terms (per `.claude/rules/testing.md` § "RED scaffolds"):

- **Test-side**: `#[should_panic(expected = "RED scaffold")]` on the
  `#[test]` / `#[tokio::test]` body, with a `panic!("Not yet implemented -- RED
  scaffold (<scenario-id> / <one-line spec>)")` body. The Red Gate Snapshot
  classifies a `panic!` / `AssertionError`-shaped failure as RED.
- **Production-side**: the new fns / match arms / helper bodies carry
  `todo!("RED scaffold: <one-line spec>")`, gated with
  `#[expect(clippy::todo, reason = "RED scaffold; lands GREEN in slice NN")]`
  (NOT `allow` — `expect` self-removes when the scaffold goes GREEN). Per project
  memory `feedback_distill_scaffold_clippy_discipline` + `feedback_panic_format_string_brace_escaping`,
  inject the clippy block at scaffold creation, and avoid `{`/`}` in `panic!`
  format strings (use `(`/`)` — `Failed { EarlyExit }` → `Failed ( EarlyExit )`).

The gate FAILS (and DELIVER must fix the test, not the production code) if any
scenario fails as:

- `IMPORT_ERROR` / unresolved type (e.g. `TxnOp::IncrementU64`,
  `RestartWorkloadResponse`, `WorkloadCommand::Restart`, `current_alloc` not yet
  declared) — a missing-scaffold bug. The Scaffold MANIFEST exists to prevent
  exactly this; DELIVER materialises every production stub before running a
  scenario that names it.
- `FIXTURE_BROKEN` / `SETUP_FAILURE` (e.g. the Lima fixture refuses to boot, a
  leaked cgroup/XDP/nft from a prior run, the `integration-tests` feature absent
  so the test binary does not compile) — infrastructure, not missing-functionality.
  See project memory `reference_pre_push_flaky_foundational_crate_lima_cleanup`
  (overdrive-core change pulls the whole integration suite; sweep leaked Lima
  state before re-running).
- `WRONG_ASSERTION` / `OBSERVABLE_NOT_AT_PORT` (e.g. a reconciler scenario
  asserting on a private View field instead of the returned `(Vec<Action>,
  NextView)` tuple) — a Universe-shape bug. The scenarios are written to assert
  only through port-exposed surfaces (the returned action set + NextView, the
  `store.get` decode, the HTTP status/body, the CLI handler's typed
  `Result<RestartOutput, CliError>` + the `render::cli_error_to_exit_code` exit
  code — a direct handler-call, NO subprocess per `crates/overdrive-cli/CLAUDE.md`,
  the `getent` name-path signal at Tier-3), so this should not arise.

## Why the classification runs in DELIVER, not DISTILL

Identical to the dial-by-name precedent (`docs/feature/dial-by-name-responder/distill/red-classification.md`).
The scaffolds NAME production types not yet in `crates/`:

- `TxnOp::IncrementU64` (a new variant on the `IntentStore` port trait) + its
  `LocalIntentStore::txn` match arm + the two test-double impls
  (`FaultInjectingIntentStore`, `CountingIntentStore`) — a scenario referencing
  the variant does not compile until the variant exists.
- `RestartWorkloadResponse` / `RestartOutcome` (api.rs), `restart_workload`
  (handlers.rs), the `/v1/jobs/:id/restart` route (lib.rs),
  `ApiClient::restart_workload` (http_client.rs).
- `Command::Workload(WorkloadCommand)` + `WorkloadCommand::Restart` (cli.rs) +
  `commands::workload` (NEW module).
- `IntentKey::for_workload_generation` (aggregate/mod.rs),
  `WorkloadLifecycleState.generation` + `WorkloadLifecycleView.observed_generation`
  + the `current_alloc` helper + the current-instance-scoped veto edit
  (workload_lifecycle.rs).

Landing the half-built surface mid-DISTILL would perturb the workspace build and
violates the project's "Implement to the design — never invent API surface"
discipline (the surface is built per ADR-0073 in DELIVER, slice by slice).
**NO file is written under `crates/` this wave.** DELIVER's RED phase
materialises each scaffold with the markers above and runs the gate.

## One-line expected classification per scenario

| Scenario | Tier | Expected RED reason | Scaffold that produces it |
|---|---|---|---|
| S-BIR-TXN-01 | 1-store | `MISSING_FUNCTIONALITY` | `TxnOp::IncrementU64` arm absent → `todo!` in `LocalIntentStore::txn` |
| S-BIR-TXN-02 | 1-store | `MISSING_FUNCTIONALITY` | same (the concurrency assertion fires on the unimplemented bump) |
| S-BIR-TXN-03 | 1-store | `MISSING_FUNCTIONALITY` | same (absent-key default-0 path unimplemented) |
| S-BIR-TXN-04 | 1-store | `MISSING_FUNCTIONALITY` | same (short-slice defensive decode unimplemented) |
| S-BIR-RESTART-STOPPED | 1 | `MISSING_FUNCTIONALITY` | `generation`/`observed_generation` fields + R4 gate absent → `todo!` in the reconciler edit |
| S-BIR-RESTART-RUNNING-STOP | 1 | `MISSING_FUNCTIONALITY` | R2 stop arm (gate the line-485 early-return on `restart_pending`) absent |
| S-BIR-RESTART-RUNNING-PLACE | 1 | `MISSING_FUNCTIONALITY` | R3 place-and-stamp arm absent |
| S-BIR-STOP-ONCE | 1 | `MISSING_FUNCTIONALITY` | R5 no-duplicate-stop arm absent |
| S-BIR-COALESCE-PLACE | 1 | `MISSING_FUNCTIONALITY` | the place-once + `observed = desired` stamp absent |
| S-BIR-COALESCE-NO-REPLAY | 1 | `MISSING_FUNCTIONALITY` | the no-replay-when-`observed == desired` arm absent |
| S-BIR-SEQUENTIAL | 1 | `MISSING_FUNCTIONALITY` | the `observed < desired` re-entry absent |
| S-BIR-REGRESSION-STOPPED | 1 | `MISSING_FUNCTIONALITY` | the current-instance-scoped veto + R1-crash branch absent |
| S-BIR-REGRESSION-RUNNING | 1 | `MISSING_FUNCTIONALITY` | same |
| S-BIR-BUG3-PRESERVED | 1 | `MISSING_FUNCTIONALITY` | the scoped veto (which must still fire on a current Operator-stop) absent |
| S-BIR-CURRENT-ALLOC | 1 | `MISSING_FUNCTIONALITY` | `current_alloc` helper absent → `todo!` body |
| S-BIR-HANDLER-404 | 1/2 | `MISSING_FUNCTIONALITY` | `restart_workload` handler absent |
| S-BIR-HANDLER-TXN | 1/2 | `MISSING_FUNCTIONALITY` | handler txn assembly absent |
| S-BIR-HANDLER-OUTCOME-RESUMED | 1/2 | `MISSING_FUNCTIONALITY` | `RestartOutcome` present⇒Resumed classification absent |
| S-BIR-HANDLER-OUTCOME-RESTARTED | 1/2 | `MISSING_FUNCTIONALITY` | `RestartOutcome` absent⇒Restarted classification absent |
| S-BIR-CLI-RESTART-SUCCESS | int (in-process) | `MISSING_FUNCTIONALITY` | `WorkloadCommand::Restart` + `commands::workload::restart` + `ApiClient::restart_workload` absent (direct handler-call, no subprocess) |
| S-BIR-CLI-RESTART-UNKNOWN | int (in-process) | `MISSING_FUNCTIONALITY` | the CLI 404→`CliError`→non-zero-exit (`render::cli_error_to_exit_code`) mapping absent |
| S-DBN-WS-STABLE | 3 | **un-ignore → GREEN** (NOT a `MISSING_FUNCTIONALITY` scaffold) | existing AT, `#[ignore]` removed + cycle swapped to the production verb (slice-04) |
| S-DBN-CHURN | 3 | **un-ignore → GREEN, AFTER a preceding production step** (see note ‡) | existing AT is still an oracle (not a `MISSING_FUNCTIONALITY` todo-scaffold), BUT the un-ignore (roadmap 03-02) is now gated on the **A1 production pump half-close-forward + T1/T2 test-model fix (roadmap 03-01)** — a clean backend FIN was invisible to v1's (B)+(C) supervision (RCA `root-cause-analysis-in-flight-churn-fail-fast-gap.md`; ADR-0070 amendment 2026-07-01). |
| S-DBN-NXDOMAIN-02-RECOVERY | 3 | **un-ignore → GREEN** | same as S-DBN-WS-STABLE |

## The oracle ATs are NOT `MISSING_FUNCTIONALITY` scaffolds

The three Tier-3 oracle ATs are **already authored and `#[ignore]`'d** (deferred
to #249). They are NOT RED-scaffolded by this feature. Their gate, run in
DELIVER's terminal slice (slice-04), is: **un-ignore (remove the `#[ignore =
"…#249…"]` string entirely — removed, not rewritten, no stale forward-pointer),
swap the `stop_and_converge` + same-spec-redeploy cycle/recovery for the
production `overdrive workload restart <id>` action, and confirm GREEN on the
pinned-6.18 appliance-kernel Tier-3 matrix** (the merge gate; dev-Lima is
necessary-but-not-sufficient — ADR-0068). No AT installs/clears a rule/key, binds
a socket, or supplies an address production does not itself install/clear/bind/supply
(CLAUDE.md vertical-slice rule).

**‡ S-DBN-CHURN exception (post-hoc, 2026-07-01).** S-DBN-CHURN is still a
genuine oracle un-ignore — it is NOT a `MISSING_FUNCTIONALITY` todo-scaffold — but
DELIVER discovered (RCA `docs/analysis/root-cause-analysis-in-flight-churn-fail-fast-gap.md`)
that it could NOT go green by un-ignore alone: a graceful `overdrive workload
restart` stops the backend with SIGTERM → the backend's socket FINs cleanly, and a
clean directional FIN was invisible to BOTH v1 liveness mechanisms ((B) self-teardown
fires only on `TransportDeath`; (C) `TCP_USER_TIMEOUT` reaps only *unacked* death).
The datapath absorbed the FIN (`PumpExit::Graceful` non-reclaim) without propagating
it to the client-facing leg-F, so the in-flight client hung to `CHURN_BOUND`. The fix
(ADR-0070 amendment 2026-07-01 — the **A1 pump half-close-forward**: on a source clean
close, `shutdown(dst, SHUT_WR)` forwards the FIN to the opposing leg) plus the
**T1/T2 test-model fix** (a long-lived full-duplex backend + a live-first-round-trip
assertion) is a **new production step (roadmap 03-01)** that precedes the un-ignore
(roadmap 03-02). This does NOT change S-DBN-CHURN's own nature (an oracle AT, not a
scaffold); it changes the *phase sizing* — the prior "un-ignore, test-only, no
production src" framing was mis-sized. The other two oracle ATs (S-DBN-WS-STABLE,
S-DBN-NXDOMAIN-02-RECOVERY) remain pure un-ignore→GREEN.
