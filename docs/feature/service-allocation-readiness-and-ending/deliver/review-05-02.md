# DELIVER Review — Step 05-02: Host reads guest listener reports into one shared board

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `05-02` — Host reads guest listener reports into one shared board |
| Iterations recorded | 1 and 2 (original review); 3 (final lint remediation) |
| Implementation commit | `18de456664707320f328177ea23bae76446c4aea` |
| COMMIT audit commit | `730f6c5bd64cbe9a081a7c02cd6beb6bb180f3c7` |
| Final-remediation commits | `493e450d900e37095813b46752c1038118dfcfbb`, `b23de5ca765193b614cf868d6dcbe98e58d13e46`, `45086aece00bfed7cdef09d3dc47d8c66789ee29` |
| Final-remediation audit commits | `9b387cb26880e3d41581a57063c53ea151f70bce`, `b0881ff6844f044622f75c8d0bb540b5227d7201` |
| Reviewer | Codex, fresh isolated replacement for final-remediation review |
| Final verdict | **APPROVED** — final lint remediation closed in iteration 3 |

## Scope and authoritative contract

Reviewed the approved roadmap step 05-02; feature-delta D13 items 2 and 10, DG-M, DG-6 and D13-S1; ADR-0268 and ADR-0272; the approved DISTILL correction and its independent review in `distill/guest-listener-board-model-remediation.md` and `distill/review-guest-listener-board-model-remediation.md`; commits `18de4566` and `730f6c5`; the step's DES events; and the changed production paths, tests, and formal model.

The pinned public surface is `BeaconMessage::Listen { protocol: Proto, port: NonZeroU16 }`, `BeaconParseError::MalformedListen { raw: String, detail: String }`, `ReportedListeners::{contains(&self, Proto, NonZeroU16) -> bool, len(&self) -> usize, is_empty(&self) -> bool}`, `GuestListenerReports::{reported, changed}`, `GuestListenerBoard::new() -> Self`, the sixth `ProbeRunner::new` parameter, and the ninth/last `VmDriver::new` parameter. The board is a per-allocation declared-set projection: it is opened before the beacon reader, only accepts new declared pairs, and is closed after probe supervision is stopped. The production `run_server` composition creates one board and passes that same instance to both consumers. The beacon reader routes `LISTEN` to the board, routes `EXIT` to the exit report, warns and drops malformed/unexpected lines, and records no listener lines after `EXIT` or EOF ends the wait.

## Iteration 1 summary

The changed production behavior matches the accepted D13 contract. LISTEN parsing uses the approved typed error and carries the complete logical input line; the reader does not send LISTEN lines through the exit channel; the board stores only declared, distinct pairs; `changed` registers before re-reading the set so an insertion between read and wait is not lost; and terminal handling calls `ProbeRunner::stop_alloc` before removing the board entry. The `run_server` composition creates one `Arc<GuestListenerBoard>`, coerces that same allocation to the read-only port for `ProbeRunner`, and passes the concrete `Arc` through `compose_vm_driver` to `VmDriver`.

The S-50 property correction is valid: the former `Duplicate(0)` could be the first report, so its no-op oracle contradicted its own expected-set insertion. The revised `Repeat` input distinguishes an actual duplicate from a first arrival and explicitly exercises both first insertion and true duplicate behavior. The Quint guard correction excludes beacon actions after `VmmExit` has closed the simulated connection while preserving the separate late-LISTEN-after-guest-EXIT-before-VMM-exit case. Neither correction weakens the declared contract or changes production behavior.

Three bounded contract defects remain. The core port source still labels the now-implemented trait and value type as a RED scaffold even though the step owns that marker and the feature delta marks the port real. `GuestListenerBoard` also implements public `Default` even though the feature delta pins `GuestListenerBoard::new()` and explicitly lists `Default` only for `ReportedListeners`. The public beacon and VM constructor documentation still describes the pre-D13 protocol/signature. These are directly visible contract/artifact defects, not inferred production failures.

## Iteration 1 findings

### F-01 — Blocking: owned RED scaffold marker remains on the implemented listener-report port

**Severity:** Blocker — the roadmap's per-step scaffold gate is not satisfied.

**Location:** `crates/overdrive-core/src/traits/guest_listener_reports.rs:4-8`

**Evidence:**

- Roadmap step 05-02 explicitly lists `traits/guest_listener_reports.rs` among its owned scaffold markers and requires the per-step scaffold gate (`deliver/roadmap.json`, step 05-02 implementation notes and the phase-01 scaffold-gate rule).
- The current module header still says `SCAFFOLD: true — RED scaffold created by DISTILL` and describes the port and value type as scaffold surface.
- The code below that marker is now the implemented `ReportedListeners` value type and `GuestListenerReports` port; the production `GuestListenerBoard` implements the port, `ProbeRunner` receives it, and the feature delta inventory calls the trait real (`feature-delta.md:3443-3447`).
- This marker cites the feature's D13 decision, so it is in scope under the gate's exact decision-id rule. `rg -n 'SCAFFOLD: true|RED scaffold'` confirms it is the remaining marker in the 05-02-owned core/worker paths.

**Required remediation:** Remove the obsolete scaffold claim and replace it with a current module description. Keep the approved type and trait surface unchanged. Recheck the step-owned paths with the roadmap's scaffold-gate search.

**Iteration 1 disposition:** Pending; return to the original 05-02 crafter for this bounded documentation correction.

### F-02 — Blocking: `GuestListenerBoard` exposes an unapproved `Default` constructor

**Severity:** Blocker — public API diverges from the pinned interface contract.

**Location:** `crates/overdrive-worker/src/guest_listener_board.rs:29-44`

**Evidence:**

- The type derives `Default` at line 30, which exposes `GuestListenerBoard::default()` as a public constructor; `GuestListenerBoard::new()` then delegates to it at lines 42-43.
- The accepted interface changes pin `GuestListenerBoard::new() -> Self`. DG-6 and D13-S1 separately and explicitly pin `Default` for `ReportedListeners`, but do not pin a `Default` implementation for `GuestListenerBoard` (`feature-delta.md:2412-2428, 2865`). The design also states that the board's write-side methods and storage are internal.
- No current caller needs the extra public constructor; the production composition and fixtures construct the board through `new()`.

**Required remediation:** Remove the unapproved `Default` implementation from `GuestListenerBoard` while retaining the exact `new() -> Self` API. No behavior or caller change is required.

**Iteration 1 disposition:** Pending; return to the original 05-02 crafter. This is an implementation/API-shape correction to the already approved design, not a DESIGN gap.

### F-03 — Medium: public rustdoc still states the pre-D13 beacon grammar and VM constructor shape

**Severity:** Medium — public contract documentation contradicts the accepted wire form and constructor signature.

**Locations:**

- `crates/overdrive-core/src/vm/beacon.rs:13-18, 91-93`
- `crates/overdrive-worker/src/vm_driver.rs:1091-1099`

**Evidence:**

- The beacon module's protocol summary lists only READY, EXEC, EXIT and SHUTDOWN and omits the newly accepted guest-to-host `LISTEN` message. `BeaconParseError::UnknownKind` likewise says the recognized kinds exclude LISTEN.
- The `VmDriver::new` rustdoc and Clippy allow reason still say ADR-0090 pins eight parameters and that the beacon intake is last. The current ninth/last `guest_listener_board` argument and exact D13-S1 signature are user-approved in feature-delta D13 item 10 and reproduced at `vm_driver.rs:1100-1110`.
- These comments are in the touched public-interface source files and can mislead callers or future protocol changes about the accepted contract. No runtime failure is claimed.

**Required remediation:** Update the module/protocol and parser-kind documentation to include LISTEN, and update the `VmDriver::new` docs and lint rationale to state the approved nine-parameter order with `guest_listener_board` last. Do not change the API.

**Iteration 1 disposition:** Pending; return to the original 05-02 crafter with F-01 and F-02.

## Contract and production-path review

### Exact API shape

| Contract | Implementation | Assessment |
|---|---|---|
| `Listen { protocol: Proto, port: NonZeroU16 }` | `beacon.rs:233-241` | Exact |
| `MalformedListen { raw: String, detail: String }` | `beacon.rs:161-171` | Exact approved semantic variant and payload |
| `ReportedListeners` exact methods and construction | `guest_listener_reports.rs:20-49` | Exact `contains`, `len`, `is_empty`, `Default`, `FromIterator<Listener>`, equality |
| Read-only `GuestListenerReports` port | `guest_listener_reports.rs:52-72` | Exact `reported` and async `changed`; no write method added |
| `GuestListenerBoard` constructor surface | `guest_listener_board.rs:29-44` | `new()` is pinned; public `Default` is not (F-02) |
| `ProbeRunner::new` sixth read-port argument | `probe_runner_boot.rs:75-90`; `probe_runner/mod.rs` | Exact |
| `VmDriver::new` ninth/last board argument | `vm_driver.rs:1100-1110` | Exact D13-S1 signature; only its stale rustdoc is finding F-03 |
| Driver/action/reconciler interfaces | Changed-call-site audit | No added driver/action/reconciler method, trait, variant, or parameter; board `Default` is separately blocked by F-02 |

No unresolved typed-error contract gap was found. The designed malformed-LISTEN cases map as follows:

| Designed failure | Approved and implemented representation | Evidence |
|---|---|---|
| Wrong LISTEN field count | `BeaconParseError::MalformedListen { raw, detail }` | `beacon.rs:393-399` |
| Unknown LISTEN protocol | Same variant; `detail` identifies the unknown protocol | `beacon.rs:401-410` |
| Non-numeric or out-of-range port | Same variant; `detail` is the integer parser's display text | `beacon.rs:412-414` |
| Port zero | Same variant with `detail = "port must be non-zero"` | `beacon.rs:415-418` |
| Malformed line received by the host reader | Warn and drop; it never reaches the exit report | `vm_driver.rs:802-809` |

The error detail supplements the approved variant and full raw line; it does not replace semantic distinctions with a string-only outer carrier. No panic, generic error remapping, or unapproved public error surface was introduced.

### Board lifecycle, beacon reader, and owner ordering

- `VmDriver::start` opens the allocation's board entry from `spec.declared_listeners` after accepting READY and before spawning `BeaconWriter` (`vm_driver.rs:1643-1646, 1673-1680`). There is no fallible return after the open; all start-error arms are before it (`:1710-1788`).
- `GuestListenerBoard::record` checks open state, declared membership, and set insertion under the entry lock; only an actual insertion calls `notify_waiters` (`guest_listener_board.rs:57-70`). `reported` is a nonblocking level read and returns empty when there is no entry (`:82-93`). `changed` registers its `Notify` waiter before checking the set, returns immediately when the projection differs, and otherwise waits for insertion (`:95-117`).
- `run_beacon_writer` routes `Listen` to `record`, routes `Exit` to the exit-line receiver, warns and drops malformed/unexpected lines, and ignores every subsequent line after a parsed Exit (`vm_driver.rs:763-825`). EOF/error ends the reader and closes its socket.
- The terminal owner calls `ProbeRunner::stop_alloc` before `GuestListenerBoard::close` (`vm_driver.rs:2106-2109`), matching D13 item 10. `stop_alloc` synchronously removes the supervisor and cancels its tokens (`probe_runner/mod.rs:355-367`). There is no reader or probe-task path that retains a closed board entry as an allocation result.
- `run_server` allocates one board and shares it across the `GuestListenerReports` coercion and `compose_vm_driver` argument (`control-plane/src/lib.rs:3310-3344`); `compose_vm_driver` passes that same board to `VmDriver::new` (`:3537-3545, 3686-3693`). The isolated probe-runner boot helper's default board is only used by its own standalone earned-trust test setup; the production `run_server` uses the explicit `compose_and_probe_runner_gate_with_reports` path.

No reachable lifecycle, ordering, retry, or cancellation defect was established. In particular, the entry is opened only after READY is accepted, and the actual terminal owner cancels supervision before close. No speculative race was promoted to a finding.

## Acceptance tests, model, and evidence

### Coverage, integrity, and external validity

The step's nine acceptance bodies are active: three LISTEN wire-form cases and six `guest_listener_board` cases. Five `quint_guest_report_host_*` conformance runs are also active. All inspected live tests carry their required Contract Shape declarations; the three source-local pure-function properties use the exact `/// CONTRACT_SHAPE: pure-function.` line. The test names do not match the banned `test_*` regex, and this feature has no DISCUSS Elevator Pitch to use as an outcome anchor.

The 8 distinct observable behaviors are: valid LISTEN round-trip; malformed LISTEN typed rejection; declared-set membership and entry lifetime; insertion-only `changed`; warning/drop and continued handling after garbage; post-EXIT line isolation from the set and exit report; Sim/production-port equivalence; and no missed insertion across seeded schedules. The budget is 8 × 2 = 16 unit tests; the 9 acceptance bodies are within budget. The five Quint conformance drivers remain a separate model-conformance layer, not extra unit tests.

The board acceptance tests drive `VmDriver::start`, the real beacon reader, and `on_alloc_terminal` through `SimBeaconIo`, then observe through `GuestListenerReports`; they do not directly author board writes. The host conformance driver also composes the production `VmDriver` and Sim beacon ports. Rust tests remain in-process and do not spawn the built Overdrive binary or emit expectation evidence.

Test integrity passes. Apart from removing assigned pending markers, the wire and board tests' behavioral assertions were not weakened or deleted. The S-50 oracle/stimulus change is an independently approved correction to a reproduced contradictory property: the previous `Duplicate(0)` could be a first insertion, while the old property expected both insertion and unchanged state. The revised property covers a guaranteed first insertion and a true repeated duplicate, and its randomized `Repeat` step only asserts no change when that pair already existed.

The host Quint model correction narrows only actions the production connection cannot accept after VMM exit. It retains the distinct late LISTEN after guest EXIT but before VMM exit, and the reviewed formal summary reports 84/84 checks with the same 13 input hashes. No invariant was weakened and no public seam or state was added.

### Verification evidence

The reviewer did not rerun tests. The following evidence is recorded in the approved DISTILL remediation artifact/review and the step run record:

- Six board acceptance bodies passed 6/6 in the independent DISTILL review; the author reports the corrected S-50 plus the complete host run set passed 5/5, including the fixed seed `0xd1acbdab` and the late-LISTEN-before-VMM-exit named case.
- The formal Quint check reports 84/84 expected outcomes and 13 input hashes; `verify-evidence` was reported passed. The independent DISTILL reviewer compared the prior and refreshed check names/outcomes and verified all 13 current hashes; it did not rerun Quint.
- The implementation handoff reports wire tests 3/3, board tests 6/6, host runs 5/5, strict worker Clippy and control-plane/sim all-target checks, changed-file formatting, and diff check passed. The earlier DISTILL-only Clippy attempt was blocked by the then-active production lint; the later implementation handoff reports the strict lint passed.
- `execution-log.json` records 05-02 RED FAIL at 10:36:57Z; GREEN FAIL at 10:59:50Z; GREEN PASS at 11:28:41Z; and COMMIT PASS at 11:32:46Z. The two failed attempts are consistent with the documented S-50 oracle and seeded Quint action-domain corrections; the passing run followed them.

I inspected both commits. The feature commit preserves Marcus as Git author and carries exactly `Co-Authored-By: Codex <codex@openai.com>` plus the step/task trailers; the audit commit does likewise. The audit commit adds only the final COMMIT event. The worktree was clean before this review artifact was written. No mutation testing was run, as required for an individual roadmap step; it remains the single final DELIVER-wave gate.

## Iteration 1 review dimensions and gates

| Dimension | Result |
|---|---|
| Exact API and typed error contract | **Fail** — approved signatures and error mapping match, but the public board `Default` implementation is not pinned (F-02) |
| Beacon reader behavior | Pass — LISTEN/EXIT classification, garbage warning/drop, and post-EXIT freeze match |
| Board entry lifecycle and owner order | Pass — open before reader; stop supervision before close |
| Production composition | Pass — one board instance is shared in `run_server` |
| Acceptance completeness/external validity | Pass — assigned wire, board, and host conformance bodies are active and use production owners |
| Test integrity | Pass — no assertion weakened; S-50 change corrects its reproduced contradictory oracle |
| Contract Shape | Pass — all active step tests declare shape; pure properties use the exact line |
| Test budget | Pass — 8 behaviors, 16-test budget, 9 acceptance bodies |
| Formal model | Pass — no weakened invariant; host input guard matches the VMM connection lifetime |
| Scaffold gate | **Fail** — F-01 remains in an owned port file |
| Public rustdoc accuracy | **Fail** — F-03 leaves the newly approved message/signature undocumented in the source summaries |
| RPP | L1 scan found F-01/F-03 documentation defects; cascade stops at L1 |
| Mutation gate | Not run — deferred to final DELIVER-wave gate |

## Iteration 1 verdict

**NEEDS_REVISION.** The implementation's reader/board lifecycle, production sharing, typed error mapping, test integrity, and formal-model corrections match the accepted contract. F-01 is a required roadmap scaffold-gate completion. F-02 is an unapproved public constructor surface and must be removed to match the exact approved API. F-03 is a bounded correction to public contract documentation in files changed by this step. Return all three findings to the original 05-02 crafter. Do not add production API or change behavior. Re-review the same step after the marker, public surface, and stale documentation are corrected.

## Iteration 2 review

| Item | Result |
|---|---|
| Remediation commit | `02e3568269a2b79e4fd9384c6980ccb2786c663c` |
| COMMIT audit commit | `88d6ff8b0efd3b0d41ea3a6c4d9f46cf4c366dfa` |
| Scope | Four production source files for the three findings; separate audit commit adds the remediation COMMIT event |
| Acceptance-test bodies/assertions | Unchanged |
| Public surface added | None |
| Commit author/trailers | Marcus retained as author; both commits have exactly the Codex co-author trailer and the 05-02 step/task trailers |

### Finding dispositions

| Finding | Iteration 2 evidence | Disposition |
|---|---|---|
| F-01 — owned RED scaffold marker | `guest_listener_reports.rs` now describes the live read-only port and construction contract without the `SCAFFOLD: true` / `RED scaffold` marker. A targeted search of the 05-02-owned beacon, port, board, and VM-driver source files finds no such marker. | **Resolved** |
| F-02 — unapproved `GuestListenerBoard::Default` | `GuestListenerBoard` now derives only `Debug`; `new() -> Self` constructs the empty map directly. The public Default implementation is gone, while all callers retain the pinned `new()` constructor. The lint exception documents the pinned constructor shape. | **Resolved** |
| F-03 — stale beacon and constructor docs | The beacon protocol summary now includes guest-to-host LISTEN, `UnknownKind` lists LISTEN among recognized kinds, and `VmDriver::new` docs and lint rationale state that D13-S1 pins the ninth/last board argument after the beacon intake. | **Resolved** |

The changes close only the findings: the remediation commit changes the three source files named by the findings plus `vm_driver.rs` for its constructor documentation, without changing acceptance tests, error mappings, lifecycle behavior, or the approved API. `git show --check` passes for both remediation and audit commits. The audit event records only COMMIT PASS at 11:52:55Z; it does not claim new RED or GREEN work.

The crafter reports the focused parser tests **3/3**, board acceptance tests **6/6**, host conformance runs **5/5**, strict worker Clippy, formatting, and `git diff --check` passed. I did not rerun those tests or broad gates; this re-review was limited to the three documented findings, their production paths, the two commit scopes, and mechanical whitespace checks.

## Final verdict at original review approval (iteration 2)

**APPROVED — iteration 2.** F-01, F-02, and F-03 are closed. The implementation retains the accepted D13 API and behavior, the owned scaffold gate is satisfied, and the public contract documentation matches the approved wire form and constructor signature. No implementation, acceptance-test, or DES phase changes were made by this reviewer; this review artifact is the only reviewer change.

## Iteration 3 — final integration lint remediation review

### Review scope and provenance

This fresh replacement review reopens only the final integration lint remediation recorded in `distill/final-dst-lint-fixture-remediation.md` and `deliver/step-05-02-final-lint-remediation.md`. It preserves the original 05-02 review and its iteration 1/2 findings and dispositions above. The reviewed commits are `493e450d` (async fixture I/O; its commit carries `Step-Id: 05-03`, matching the step that activates this wiring test), `b23de5ca` (private scanner recognition), `9b387cb` (the scanner remediation COMMIT audit), `45086aec` (narrow Clippy allowances), and `b0881ff` (the allowance remediation COMMIT audit).

The current worktree also contains unrelated edits and parallel DISTILL test/model alignment work, including the guest-flow `fwd_pair` model and the `step-05-07-sigkill` spike. I did not modify or commit those files. This review artifact is the only reviewer-owned file.

### Findings and dispositions

| Item | Review evidence | Disposition |
|---|---|---|
| Async shared-board fixture filesystem calls | `493e450d` changes the five `std::fs` sites called out by the canonical DST lint to awaited `tokio::fs` operations: staging/data/operator-config directory creation and the two stand-in file writes. It also makes the fixture's private `read_ca` read async. The patch changes setup sequencing only where needed to await I/O; it leaves the wiring test's identity, inputs, expected-failure behavior, exact startup `Pass` oracle, and `CONTRACT_SHAPE: bounded-change.` declaration intact. | **Resolved** |
| Earned-Trust helper recognition | The feature-delta D13/DG-6 contract requires one shared `GuestListenerBoard` in production composition. `run_server` constructs one board, gives a clone as `GuestListenerReports` to `compose_and_probe_runner_gate_with_reports`, and passes the same board through `compose_vm_driver` to `VmDriver::new` (`lib.rs:3299-3334, 3530-3545, 3686-3693`). The reports helper builds `ProbeRunner` with that port and awaits `runner.probe()` (`probe_runner_boot.rs:75-92`). Thus the original `ProbeGateHelperUnused` failure was a stale private scanner name after the approved composition changed, not a missing runtime gate. The scanner now compares against the two exact helper identifiers; the pre-existing `#[cfg(test)]` exclusion and separate `.probe()` body check remain. The regression suite exercises the actual `lib.rs`, the exact `with_reports` call, a missing call, test-only calls, and a `_disabled` lookalike that must remain a violation. | **Resolved** |
| Contract Shape and scoped lint allowance | The two new source-local scanner properties retain the exact `/// CONTRACT_SHAPE: pure-function.` line. `45086aec` adds `#[allow(clippy::doc_markdown, reason = "exact per-test contract declaration")]` only to those two test functions. It adds no module/file-wide allowance and changes no scanner assertion or behavior. | **Resolved** |

### Accepted contract and test-integrity checks

- The reviewed changes add no production API, test fixture API, wire field, error variant, or public configuration. `BeaconMessage::Listen { protocol: Proto, port: NonZeroU16 }` and `BeaconParseError::MalformedListen { raw: String, detail: String }` remain the approved D13/DG-M surface; the changed commits do not touch their implementation or mappings.
- The shared-board production composition remains one board passed to the probe runner and VM driver. The test fixture still proves the externally observable startup `Pass` through `run_server`, its beacon input, and the allocation snapshot; it does not hand-install the expected board state.
- The fixture commit's diff contains no assertion edits or test removal/skip. The scanner tests add observable scanner-result assertions, including the negative lookalike case. No testing-theater or weakened-oracle issue was found.
- The `doc_markdown` exception is bounded to the two mandated exact contract-marker declarations. No broader lint suppression, unrelated formatting cleanup, or API adjustment is required.

### Verification evidence and DES audit

I reviewed the retained evidence and commit history rather than rerunning the native Metal lane:

- Fixture evidence records the five DST-lint diagnostics removed and the selected `a_listen_line_on_the_drivers_beacon_completes_the_probe_tasks_declared_set` run passing 1/1 (`17727855873b17020af8ca15`) after lane preparation. The earlier `lane_not_prepared` attempt did not reach the test body.
- The fixture-only canonical lint attempt `2efb5fe2c8c0b64453015293` is correctly recorded as still failing on the one feature-caused stale `ProbeGateHelperUnused` recognition, after the five async filesystem diagnostics were gone. The evidence does not claim that this intermediate run passed.
- The scanner regression was reproduced RED on the real production `lib.rs` (`0/1`, expected `ProbeGateHelperUnused`), then the focused `probe_gate_` suite passed 10/10. The full canonical Metal `dst-lint` passed as run `dbf2700f5458905e4584797e` after the scanner fix.
- Strict Clippy first reported exactly the two `doc_markdown` diagnostics on the required marker lines. The function-local allowances were followed by the recorded strict xtask Clippy pass `263862225a837f0b81f17bb5`; direct rustfmt for `xtask/src/dst_lint.rs` passed.
- `execution-log.json` retains separate 05-02 RED FAIL / GREEN PASS / COMMIT PASS sequences for the scanner correction and the Clippy follow-up. The scanner COMMIT audit is `9b387cb`; the Clippy COMMIT audit is `b0881ff`. The DISTILL fixture note accurately records that fixture remediation had no DELIVER DES phase event. No event claims the failed intermediate lint passed.
- The full-package xtask format check still reports the documented pre-existing difference in unchanged `xtask/src/metal_run_isolation_conformance.rs:519`; the edited scanner file passes its direct format check. This unrelated cleanup is outside the approved bounded remediation.
- Mutation testing was not run. The user explicitly waived mutation testing for this remediation/step, so no mutation or D17 mutant evidence is a remaining gate.

All five commits preserve Marcus as Git author and carry exactly `Co-Authored-By: Codex <codex@openai.com>`; none adds Claude/Anthropic attribution. The commits and `git show --check` evidence show no whitespace errors. I did not alter the acceptance fixture, scanner, DES log, implementation, or any unrelated dirty work.

### Iteration 3 review dimensions

| Dimension | Result |
|---|---|
| Exact D13 API and typed-error contract | Pass — the remediation adds no interface surface and preserves `Listen` / `MalformedListen` unchanged |
| Shared-board composition and Earned-Trust gate | Pass — the production composition still passes the same board to both consumers and reaches the helper that awaits `ProbeRunner::probe()` |
| Scanner recognition and regression quality | Pass — exact approved names accepted; absent/test-only calls and a lookalike rejected; prior internal `.probe()` gate remains |
| Fixture test integrity and Contract Shape | Pass — five blocking calls are awaited; the extra CA read is awaited; assertions and exact startup `Pass` oracle are unchanged; required marker remains |
| Clippy allowance scope | Pass — two function-local allowances only |
| Verification and DES honesty | Pass — intermediate failures, final passes, and audit commits are distinguished accurately |
| Commit attribution and scope | Pass — author retained, exact Codex trailer, no unrelated files included in the reviewed commit scopes |
| Mutation gate | Not run — explicitly waived by the user for this step |

### Final remediation verdict

**APPROVED — iteration 3.** The async fixture change preserves every assertion and its bounded startup-`Pass` outcome. The scanner now recognizes the two approved Earned-Trust helper names while retaining the separate helper-body check and rejecting lookalikes. The two required Contract Shape declarations remain exact with only function-local `doc_markdown` allowances. The final canonical lint and strict Clippy evidence pass, DES records accurately distinguish RED, GREEN, and COMMIT, and the approved D13 API, shared-board wiring, and typed error contract remain intact. No further remediation is required for this bounded scope.
