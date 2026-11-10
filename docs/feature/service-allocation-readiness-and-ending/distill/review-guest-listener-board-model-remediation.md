# Independent DISTILL review — guest listener board and host-report model

## Metadata and verdict

| Item | Value |
|---|---|
| Scope | S-50 board property stimulus/oracle and `guest_report_host` conformance input guards in `guest_report.qnt` |
| Author record | `guest-listener-board-model-remediation.md` and retained `.context/step-05-02-*` Metal logs |
| Protected work | Original step 05-02 crafter's dirty board/reader/production composition, activated tests, DES phases and commit |
| Verdict | **APPROVED** for these bounded DISTILL test/model corrections; the crafter's production lint and GREEN gate remain separate |

## S-50 — first valid report is not a duplicate

**Reproduced failure.** The old property generated `steps = [Duplicate(0)]` with no after-close steps. That label was the **first** delivered `LISTEN tcp 8080`; the production reader/board correctly inserted the declared pair. The old test both added it to its expected set and asserted that the set equalled the prior empty set. This self-contradiction cannot establish a board defect. D13 item 10 requires the first declared insertion to grow the set and only an already recorded pair to be a no-op.

**Correction.** The generated operation is now `Repeat(i)`, an attempted repeat that may be a first arrival. Before each such operation, the test computes `actual_duplicate` from membership in the prior expected set and requires unchanged output only for a real duplicate (`guest_listener_board.rs:329–449`). Every generated case also sends an undeclared first line and requires an empty set, sends the first declared pair and requires insertion, then delivers that pair again and requires no change. The subsequent randomized declared/undeclared/other-allocation sequence still checks exact reported-set equality, containment in the declared universe, monotonic growth, duplicate no-op, other-allocation isolation and no post-close resurrection. The real `VmDriver`/beacon reader and `GuestListenerBoard` are still driven; no fixture method authors a report directly. The S-50 test keeps `/// CONTRACT_SHAPE: bounded-change.` and the original crafter's activation marker removal.

## Host conformance — VMM exit closes this beacon input lifetime

**Reproduced failure.** With `QUINT_SEED=0xd1acbdab`, the prior host model allowed `AppendListen(udp53)` after `VmmExit` because it tested only `not(eof)`. The production conformance driver had called `SimVmm::exit_process`, notified `SimBeaconIo`, released the exit gate and waited for the real exit observer's terminal row. Its next `SimBeaconIo::guest_write` returned false: the beacon connection was no longer open, so the driver rejected `AppendListen` as not enabled (`.context/step-05-02-guest-report-seed-before.log:3168–3196`; `quint_guest_report_host.rs:529–572`). This was an action-domain mismatch, not a reported-set or exit-row projection difference.

**Correction and exact distinction.** In the host-only `guest_report_host` module, `HostRead`, `AppendListen`, `AppendExit`, `HostGarbage` and `BeaconClose` now require the existing `exitrep == "none"` phase (`guest_report.qnt:261–314`). Only `VmmExit` moves `exitrep` off `none`; the guard therefore excludes further beacon input/read grants **after the VMM exit and its terminal observation**, when the conformance connection is closed. It does **not** exclude a late `LISTEN` after the guest's `EXIT` line while the connection remains open and before VMM exit. The retained named `host_listen_after_exit_is_ignored` run appends `EXIT`, then `LISTEN`, reads `EXIT`, reads the late line without growing `rset`, and only then invokes `VmmExit` (`guest_report.qnt:357–367`). The exact boundary is guest `EXIT` versus **VMM process exit**; the two must not be collapsed. `BeaconClose`/EOF before VMM exit remains permitted and its partial-set named run stays active.

The full guest-side `guest_report` module, host driver action map, `GuestListenerReports::reported` and `ObservationStore::alloc_status_row` projections, and declared-set/drop/exit safety obligations are unchanged. No new state, action, API, Sim seam, reconnect behavior or production owner was introduced. This bounded guard narrows only host input operations that the reproduced production connection could not execute; it is not a policy change to ignore lines that arrive before VMM exit.

## Verification and disposition

| Evidence | Result and limit |
|---|---|
| S-50 before/after | The shrunk first-`Duplicate(0)` contradiction was reproduced by the author. My independent focused Metal run of the six `guest_listener_board` acceptance bodies passed **6/6**, exit 0 (run ID `53650d3037d398ab937023cf`). The property now guarantees both first-insertion and true-duplicate stimuli. |
| Host conformance | Author's retained logs show the original seed failed on the post-VMM-exit append, then passed **1/1** after the guard; four named runs plus random host conformance passed **5/5**. The late-LISTEN-before-VMM-exit named run remains in that selection. |
| Formal evidence | Recorded full feature check passed **84/84** (`.context/step-05-02-quint-formal-after.log`). I compared the current and prior evidence summaries: all 84 check names and their expected/actual holds-or-violation outcomes are unchanged; current summary has 84 passes and all **13** recorded input hashes match current spec files. The author reports `verify-evidence` passed after pulling the refreshed evidence; I did not rerun Quint. |
| Static gate | Changed-file rustfmt and `git diff --check` passed. Strict worker Clippy was attempted but remains **blocked by the crafter's production** `guest_listener_board.rs:55` `significant_drop_tightening` lint; the 6/6 acceptance tests compiled and ran. This review does not call the strict lint gate clean. |

The temporary Cargo seed override is restored and `.cargo/config.toml` is clean. My review made no production/test/model changes, DES events or commit; `git diff --check` passed. **Verdict: APPROVED for this bounded DISTILL correction.** The original step 05-02 crafter must fix its production lint, rerun GREEN and commit before the independent DELIVER step review. The revised S-50 and host model still exercise the accepted insert/no-op/late-line/exit boundaries through production owners.
