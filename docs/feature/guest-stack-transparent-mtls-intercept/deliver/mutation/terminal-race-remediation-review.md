# Terminal Race Mutation-Baseline Remediation Review

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review scope | Final mutation-baseline `StopAllocation` / exit-observer terminal-race remediation only |
| Reviewed commit | `f2a66c91884cbf63508b316ee2fa087830e0e97d` |
| Parent | `5d359600d9321abb25e54492e4d2be875397f245` |
| Range | `5d359600d9321abb25e54492e4d2be875397f245..f2a66c91884cbf63508b316ee2fa087830e0e97d` |
| Subject | `fix(control-plane): close stop exit-observer race` |
| Required trailer | `Feature-Id: guest-stack-transparent-mtls-intercept` — present and exact |
| Review iteration | 1 |
| Verdict | **NEEDS_REVISION** |

## Review boundary

This review is limited to the three-file terminal-race remediation: the
`StopAllocation` terminal compound-write path, the exit observer's
same-attempt Job fence, and the deterministic race partitions added in the
acceptance/unit tests. It does not re-review the previously approved mTLS,
network, VM-reclamation, or streaming architecture. No production or test
source was modified by the reviewer, and mutation testing was not run.

The reviewed range changes exactly the requested files:

- `crates/overdrive-control-plane/src/action_shim/mod.rs`;
- `crates/overdrive-control-plane/src/worker/exit_observer.rs`;
- `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs`.

The range contains 390 insertions and 90 deletions. The direct parent,
conventional subject, required feature trailer, and three-file scope are
correct. The pre-existing dirty `AGENTS.md` was not touched.

## Design and API boundary

The remediation preserves the approved public and persistence boundary. It
adds no public method, type, variant, parameter, record, REST/OpenAPI shape, or
wire shape. `write_alloc_lifecycle` remains the sole allocation-current author
and direct lifecycle broadcast remains best effort. The terminal effect order
is still:

1. await process quiescence;
2. await allocation mTLS stop;
3. tear down structural network state and release its slot;
4. call `Driver::on_alloc_terminal`;
5. accept terminal current plus occurrence;
6. release supervision;
7. remove the process-local driver route;
8. broadcast the accepted occurrence.

The implementation correctly re-reads the authoritative current row after
cleanup, derives a strictly newer proposal from that row, and retries when an
equal-timestamp exit observation wins. On the finite intended race, rejected
writes append no occurrence, emit no direct event, and do not remove the route;
the accepted retry authors one Reconciler occurrence and then performs the
terminal tail once. Read and write errors propagate as the existing typed shim
or observation error and explicitly release supervision.

The exit observer also correctly consults the existing
`allocation_attempt_transition` fence before classification. A terminal Job
claim is an exact no-write/no-broadcast event, while the current implementation
continues to apply exit observations to Services and to Job rows with
`terminal: None` (including Platform Reclamation). The fence adds no second
source of lifecycle truth.

## Correctness analysis

| Concern | Result | Evidence |
|---|---|---|
| Post-cleanup authoritative re-read | PASS | `action_shim/mod.rs:2498-2536` re-reads before each proposal and derives the counter from the winner |
| Equal-timestamp loser retry | PASS for one finite contender | `action_shim/mod.rs:2568-2570`; focused rejection partition passes |
| Cleanup and supervision order | PASS on ordinary success and returned errors | cleanup precedes the loop; every explicit read/write error releases; accepted/exact-terminal completion releases at `:2579-2581` |
| Source/terminal/occurrence values | PASS in exercised partitions | final row/occurrence are `TransitionSource::Reconciler`, carry the operator terminal, preserve `from = State(Terminated)`, and yield exactly three occurrences |
| Route and direct broadcast placement | PASS by static inspection | both remain inside `if occurrence.is_some()` at `:2592-2594` |
| Late Job fence | PASS | `exit_observer.rs:463-471`; terminal Job handler test proves current and occurrence count remain unchanged |
| Late Service / reclamation behavior | Correct in source, incompletely pinned | canonical predicate gates only `Job && terminal.is_some()`; no exit-observer test proves the Service complement |
| Cancellation safety | FAIL | finding TRR-02 |
| Retry termination/bound | FAIL | finding TRR-01 |
| Assertion weakening/testing theater | PASS | no test was skipped, deleted, made tautological, or redirected around the production dispatcher; both new race tests fail against the parent implementation for the intended reason |
| Contract Shape / Outcome anchors | PASS | both new acceptance tests use exact `bounded-change` plus `Outcome anchor`; the source-local handler test has the exact Contract Shape declaration |

## Test integrity and coverage

The two action-shim tests are honest deterministic partitions. One parks
`Driver::stop`, lands the exit winner before cleanup completes, and proves the
terminal proposal rebases from counter 10 to counter 11. The other parks the
first compound write, accepts an equal-timestamp Driver/Exec occurrence, rejects
the stale terminal, and proves the retry lands the third occurrence. The
source-local exit-observer test proves the immutable terminal Job case is a
true zero-delta operation. The shared pending-write tests retain their existing
accepted/error assertions and now keep Stop rows explicitly Job-kind; no prior
assertion was weakened.

The focused additions remain within a reasonable test budget: three focused
tests cover the two race windows and the terminal fence. They use the
production dispatch/handler boundaries and port doubles only at observation
store and driver boundaries. There is no zero-assertion, circular-oracle,
mock-dominated, or always-green test.

Coverage is nevertheless incomplete for the exact negative and abandonment
properties this change introduces. TRR-03 records the missing complements.

## Findings

### TRR-01 — repeated LWW rejection has no termination or abandonment bound

- **Severity:** High
- **Dimensions:** liveness, bounded retry, forward progress, supervision
  ownership
- **Evidence:** `crates/overdrive-control-plane/src/action_shim/mod.rs:2498-2577`;
  `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:692-727,1144-1153`

`Ok(None)` immediately repeats an unbounded `loop` with no attempt ceiling,
deadline check, backoff, or abandonment result. Re-reading the latest winner
does guarantee that the next proposal dominates the snapshot just read, but it
does not make the read-modify-write atomic and does not guarantee that this
writer wins: another valid writer may advance the row again before every
compound write. A conforming observation-store implementation may therefore
return `Ok(None)` repeatedly. The action retains supervision throughout and
the production convergence loop awaits actions sequentially, so persistent
contention can starve this allocation and stall later convergence work.

The new store double can reject exactly once and then delegates to a store
which necessarily accepts the retry. It cannot expose repeated rejection or
prove a bound. The intended one-exit-observer race makes progress, but the
code's contract is broader than that fixture and contains no structural
statement of the finite-contention assumption.

**Required remediation:** pin a bounded rejection policy and its exhaustion
disposition, then test a store that keeps returning `Ok(None)`. Exhaustion must
release supervision exactly once, retain the driver route, append and broadcast
nothing, and leave the level-triggered action replayable. The approved design
sanctions no new public error variant and does not say whether exhaustion
returns an existing error or yields for later convergence, so that exact
disposition must be fixed by DESIGN rather than improvised in implementation
review.

### TRR-02 — cancellation can strand supervision or commit without the terminal tail

- **Severity:** High
- **Dimensions:** cancellation safety, atomic-effect composition, route/event
  completeness
- **Evidence:** `crates/overdrive-control-plane/src/action_shim/mod.rs:2483-2594`;
  `crates/overdrive-store-local/src/observation_backend.rs:688-704`;
  `.claude/rules/rust.md` cancellation-safety rule

After cleanup and `on_alloc_terminal`, supervision is released only by
explicit result branches. Dropping the dispatch future while it awaits the new
authoritative read or a compound write skips every release branch. At the read
boundary that leaves a quiescent, cleaned allocation supervised indefinitely.

The production local store makes the write boundary more serious: its redb
transaction runs in `spawn_blocking`. Dropping the outer future does not cancel
that blocking transaction. It can commit the terminal current plus occurrence
after the action future is gone, while the action never releases supervision,
removes the route, or sends the direct lifecycle event. The store's own
subscription emit is also after the blocking join, so it is skipped on this
cut. A later Stop replay sees the exact terminal at the initial preflight and
returns before repairing any of those process-local tail effects.

The production convergence loop deliberately observes graceful shutdown
between ticks, which reduces ordinary shutdown exposure, but it does not make
the public async operation cancellation-safe and does not satisfy the
repository rule that a partially-applied mutation tolerate cancellation at
every await.

**Required remediation:** a private synchronous drop guard is the smallest
way to make supervision release total, but it does not resolve the
commit-ambiguity, route, subscription, and direct-broadcast cut. DESIGN must pin
the existing compound commit as a cancellation-safe owned/shielded completion,
or pin a structurally uncancellable operation boundary, without adding public
API or a parallel persistence system. Add a deterministic cancellation test at
both the post-cleanup read and in-flight local compound-write boundaries; assert
one release and either no accepted occurrence with the route retained, or one
accepted occurrence with the complete accepted-write tail.

### TRR-03 — the tests do not pin the negative Service/reclamation or rejected-write side effects

- **Severity:** High
- **Dimensions:** acceptance coverage, mutation resistance, bounded-change
  complements
- **Evidence:** `crates/overdrive-control-plane/src/worker/exit_observer.rs:467-470,742-800`;
  `crates/overdrive-control-plane/tests/acceptance/action_shim_crash_observability.rs:994,1092-1123,1144-1284`

The handler test proves only `Job + terminal Some -> NoWrite`. It does not prove
that a terminal Service still applies an exit observation or that a reclaimed
Job with `terminal: None` still applies through this handler. A widening mutant
which fences every terminal row would preserve the new positive test while
breaking Service restart semantics.

The rejected-write partition drops the lifecycle receiver and never inspects
the `AllocDriverIndex`. It proves the final retry was accepted, but not that the
intermediate rejection emitted no direct lifecycle event or retained the route
until acceptance. Nor does any new test hold the second write pending so those
intermediate effects can be observed. These are named invariants of the change,
not implementation details.

**Required remediation:** add the minimal table-driven exit-handler complements
for terminal Service and terminal-free reclaimed Job rows, asserting exact
current, source, terminal, and occurrence cardinality. Extend the rejection
seam to park the accepted retry and assert zero broadcasts plus the retained
route before acceptance, then exactly one Reconciler broadcast and route
removal after acceptance. Preserve the current production driving boundary and
Contract Shape/Outcome declarations.

### TRR-04 — exit-observer module documentation still promises a write for every event

- **Severity:** Low
- **Dimensions:** contract documentation, readability
- **Evidence:** `crates/overdrive-control-plane/src/worker/exit_observer.rs:1-34,302-320,452-457`

The module summary says every exit event is mapped and written and calls the
broadcast a permanent record. The new terminal-Job path intentionally authors
neither a row nor a broadcast, and the approved R0 contract explicitly keeps
direct broadcast best-effort with no replay/exactly-once guarantee. The
function-level documentation is accurate, but the public module narrative and
`RetryOutcome` summary are now contradictory.

**Required remediation:** describe the terminal-Job no-write exception at the
module and retry-outcome summaries and call the accepted occurrence, not the
ephemeral broadcast, the durable record.

## Independent verification

| Command | Result |
|---|---|
| Focused race/fence/supervision selection through Lima | PASS — 5/5; nextest run `97667fc0-d9da-44e0-ad99-d74f58c3360b` |
| Original `s_02_05_anti_scenario_no_is_running_with` once | PASS — 1/1; run `4fec46f3-1ee4-4200-beac-0caf9b040e6a` |
| Original S-02-05 repeated 20 times in foreground | PASS — 20/20; the loop exited 0 |
| Full `overdrive-control-plane` with `integration-tests` | PASS — 797/797, one passing test marked leaky, three skipped; run `ff90b0f5-7264-4d7c-8357-c864cacc7734` |
| Full `job_kind_streaming` module | PASS — 8/8; run `85e57492-2623-4318-b202-c8250553ca48` |
| Existing `terminal_vm_job_rejects_every_reopening_event` property | PASS — 1/1; run `dca48995-e1ae-4110-a376-aa25cf02ee4d` |
| Lima clippy for control-plane and CLI, all targets, `integration-tests`, `-D warnings` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `cargo xtask dst-lint` | PASS |
| Reviewed-range `git diff --check` | PASS |
| Mutation testing | NOT RUN — correctly excluded from this review |

Green execution confirms that the finite race now converges and the original
streaming regression is stable in the exercised schedule. It does not close
the unbounded-contention, cancellation, or negative-complement findings above.

## Verdict

**NEEDS_REVISION.** Commit
`f2a66c91884cbf63508b316ee2fa087830e0e97d` fixes both observed finite race
partitions and applies the correct Job-only terminal fence without public API
drift. It is not ready for acceptance because the new retry loop has no bound,
the post-cleanup commit interval is not cancellation-safe, and the tests do not
pin the Service/reclamation or rejected-write route/broadcast complements.
TRR-01 and TRR-02 require the missing exhaustion/cancellation contract to be
pinned by DESIGN before implementation remediation; no reviewer-invented
terminal ownership or error surface is authorized.
