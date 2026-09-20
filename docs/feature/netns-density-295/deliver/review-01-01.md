# DELIVER Review — `netns-density-295` / `01-01`

## Metadata

- Reviewer: software-crafter reviewer (isolated step reviewer)
- Iteration: 1
- Commit: `67f855af483b0f2be524a50545adc2a6151a4621`
- Scope: the complete 25-file commit diff and the production caller/owner paths it changes
- Authoritative contracts: `feature-delta.md` F-01, RUN-295-B, C-295-G, D-295-DISTILL-1, D-295-DISTILL-8; `roadmap.json` step `01-01`

## Summary

The dependency-neutral EXEC gate and the seven-argument `VmDriver::new` cut match the accepted public shape, and the gate implementation itself passes the six authored core gate properties when explicitly run as ignored tests. The constructor and injected-server compiler fallout is complete and neutral, the Host owner remains one private `todo!("RED scaffold: ...")` owner, and the worker release scenarios remain ignored.

`CHANGES_REQUIRED`: the commit also implements and composes the retained supervisor behavior owned by later step `03-03`. That out-of-step implementation has a proven classification failure and creates a detached task on helper boot errors. The retained supervisor field is additionally an `Option` with a pending compatibility branch, rather than the exact private field named by the accepted design.

## Evidence and verification

- Commit attribution and DES phase order were mechanically verified by the orchestrator: the original author is retained, exactly one Codex co-author trailer and one `Step-Id: 01-01` trailer are present, and RED → GREEN → COMMIT events are ordered and PASS.
- The entire commit diff was inspected. `git diff --check 67f855af^ 67f855af` passes.
- The exact `VmDriver::new` declaration is seven arguments, with `guest_network_exec` immediately before `layout`; no `Driver` trait method or compatibility constructor was added.
- The exact `GuestNetworkProvisioner` and `SharedGuestNetworkOwner` doc-hidden traits are present, with one private `HostSharedGuestNetworkOwner` scaffold. No second production owner or public fault hook was added.
- All three worker EXEC-release bodies remain `#[ignore]`; the core gate bodies also remain ignored. No S-ND295 scenario was activated by the commit.
- `cargo xtask lima run -- cargo test -p overdrive-core --test acceptance netns_density_exec_gate -- --ignored --nocapture`: **6 passed**. This was an explicit review run of still-ignored bodies and does not change their activation state.
- `cargo xtask lima run -- cargo test -p overdrive-control-plane --lib actual_tokio_exit_matrix_fail_stops_before_returning_the_exact_snapshot -- --ignored --nocapture`: **failed**, with `left: RequestChannelClosed` and `right: SupervisorReturned` at `crates/overdrive-control-plane/src/lib.rs:1517`. This is the concrete regression cited below.
- `cargo xtask lima run -- cargo test -p overdrive-worker --test acceptance netns_density_exec_release -- --ignored --nocapture`: two ignored bodies pass and the backpressure/cancellation body remains failing; it is explicitly pending the later worker-release activation step and is not used as a finding for `01-01`.
- The orchestrator reports the required Lima workspace check, clippy, and worker/control-plane/sim suite green. No mutation testing was run.

## Design and API comparison

### Matches

- `GuestNetworkExecWiring::new(clock)` creates one private `Mutex`/`Notify` state in `BootClosed` with zero claims; gate, supervisor, and claim fields are private and the capability types are not `Clone`.
- `claim_release` registers a waiter before unlocking in `BootClosed`/`Recovering`, claims only in `Open`, refuses in `FailStop`, and `GuestNetworkExecClaim::Drop` decrements under the same lock and wakes waiters.
- Supervisor transitions and the immutable recovery projection match the accepted state vocabulary and return shapes. Core property coverage exercised these transitions successfully.
- `VmDriver::release_for_exit_emission` claims before taking `pending_exec` or `gate_sender` and retains the claim through the beacon writer outcome. The public `Driver` trait is unchanged.
- The exact seven-argument constructor and both exact injected-server helper signatures are present. All inventoried constructor/helper callers compile with neutral wiring.
- The doc-hidden owner traits, one private host scaffold, typed shutdown vocabulary, one `CliError::SharedGuestNetworkFailStop` variant, and thin CLI delegation have the requested names and types.

### Divergences

The accepted D8 owner contract says `ServerHandle` owns exactly
`shared_network_supervisor: SharedNetworkSupervisorHandle`, and step `03-03`
owns the actual retained-task classification/recovery behavior. The commit
instead adds `Option<SharedNetworkSupervisorHandle>` at
`crates/overdrive-control-plane/src/lib.rs:1232`, a `None => pending()` fallback
at `:1818-1824`, and a live supervisor task/classifier in `:1267-1312` and
`:2584-2597`. This is neither the exact private shape nor the bounded
compile-time-only scope of step `01-01`.

## Findings

### F-01 — Blocking: later-step retained-supervisor behavior and optional compatibility path landed in `01-01`

**Evidence:** Step `01-01` explicitly leaves retained supervisor recovery for
roadmap step `03-03`; its scope is the exact API cut, private scaffolds, and
thin delegations. The commit replaces the accepted RED scaffold at
`crates/overdrive-control-plane/src/lib.rs:1267-1270` with a full join/channel
classification and `exec.fail_stop` implementation at `:1267-1305`, spawns a
supervisor task in `run_server_with_obs_and_drivers` at `:2584-2597`, and adds
an `Option` field plus a permanently pending fallback at `:1232` and
`:1818-1824`. The accepted design requires the exact private field
`shared_network_supervisor: SharedNetworkSupervisorHandle` and assigns the
retained supervisor implementation to step `03-03`.

**Why this blocks:** It introduces later-step behavior and an optional
compatibility path not named by the approved step. It also means the injected
`shared_guest_network` is discarded (`let _ = &shared_guest_network`) while a
placeholder task is composed, so the commit does not represent a thin,
faithful owner handoff.

**Bounded remediation:** Restore the `01-01` boundary: retain only the exact
private/API scaffolding required to compile the two delegations and leave the
retained supervisor method/task behavior as the approved RED scaffold for
`03-03`. Do not keep an `Option`/pending fallback or invent a compatibility
owner. If the exact private field cannot be made compile-time-only without a
different shape, surface that as a DESIGN gap rather than choosing an
optional shape.

**Disposition:** `CHANGES_REQUIRED`; return this step to its original crafter
for removal of the out-of-step supervisor composition.

### F-02 — Blocking: the landed supervisor classification is reversed and loses the typed failure source

**Evidence:** The accepted D8 contract requires the retained join branch before
the request-receiver branch so a terminal task result cannot be masked by the
sender being dropped. The commit's biased `tokio::select!` polls
`request_rx.recv()` first at `crates/overdrive-control-plane/src/lib.rs:1274-1284`.
When the task returns, its `_request_owner` is dropped at the same terminal
transition, so both branches are ready and the receiver branch wins. The
bounded real owner-path regression run named above fails at
`src/lib.rs:1517`: the returned task is classified as
`RequestChannelClosed`, not `SupervisorReturned`. The same match drops
`SharedNetworkSupervisorError` with `Ok(Err(_))` at `:1287`, contrary to the
accepted requirement to retain/emit the typed source before returning
`SupervisorFailed`.

**Bounded remediation:** Because this behavior is out of step, remove it with
F-01's later-step code. When step `03-03` owns the implementation, poll the
retained join branch first and preserve the exact typed source; do not add a
new error or public hook.

**Disposition:** `CHANGES_REQUIRED`; the failed ignored regression is concrete
evidence that the currently landed implementation cannot be accepted.

### F-03 — Blocking: a supervisor task detaches on any helper boot error

**Reachable production path:** `run_server_with_obs_and_driver` delegates to
`run_server_with_obs_and_drivers` at `crates/overdrive-control-plane/src/lib.rs:2552-2558`.
That function spawns the parked task at `:2584-2592` before the first fallible
production boot call, `start_local_node(...).await?`, at `:2618-2620` (and there
are further fallible `?` paths). If any of those paths returns an error, no
`ServerHandle` is produced, so the only owner that would call
`SharedNetworkSupervisorHandle::shutdown` at `:2000-2002` never exists. The
local `JoinHandle` is dropped and the task remains parked forever on
`task_shutdown.cancelled().await` while the process/runtime remains alive.

**Why this blocks:** Task submission is not effect completion, and this is a
concrete detached async effect on a normal boot-refusal path, not a theoretical
future cancellation.

**Bounded remediation:** Remove the task composition from `01-01`; the later
`03-03` owner must only publish a retained task after the approved boot path
can own its shutdown, or cancel and await it on every pre-handle error path.
Do not add a detached observer or a second owner.

**Disposition:** `CHANGES_REQUIRED`; no later step may advance while this
detached task path remains in the step-01 commit.

## Test-honesty and TDD assessment

- No assertion was weakened or deleted in the changed acceptance files. The
  constructor/helper additions are neutral wiring; the three new worker module
  bodies remain reasoned `#[ignore]` with their `CONTRACT_SHAPE` declarations.
- The core gate property, finite transition, and fail-stop tests all carry the
  required rustdoc Contract Shape declarations and passed when explicitly run
  as ignored. They remain inactive as required by the step.
- The worker ignored suite's one failing backpressure body is not treated as a
  `01-01` defect: its ignore reason names the later worker-release activation
  step, and activating that body is explicitly out of scope here.
- No test harness spawns the production binary or runs mutation testing. The
  review's ignored-test commands only exercised authored in-process tests for
  evidence; they did not alter source activation markers.

## File-scope assessment

The four files outside the roadmap's guidance list are tightly related to the
mandatory signature/API cut: `overdrive-cli/src/http_client.rs` carries the
one required error variant, the shared-network startup integration fixture and
conformance harness supply the new owner/wiring arguments, and
`netns_density_exec_release.rs` must type-check once its module is visible.
The fixture's current legacy `AllocationSpec` fields are a neutral
compile-time representation while the grouped `network` cut belongs to step
`02-01`; no acceptance assertion or ignore marker was weakened. No unrelated
production behavior was found outside the retained-supervisor scope described
in F-01.

## Verdict

# CHANGES_REQUIRED

F-01, F-02, and F-03 are unresolved blocking findings. No later roadmap step
may start until the original crafter removes the out-of-step supervisor
behavior/task composition (or a separately approved DESIGN remediation pins a
different shape), the exact private API shape is restored, and this step's
reviewer re-runs the bounded regression evidence and records an APPROVED
iteration in this artifact.

## Iteration 2 — remediation commit `cc752bcf0cd83707c06071dcc88b2b0bbfd88b37`

### Scope and verification

The remediation commit is limited to
`crates/overdrive-control-plane/src/lib.rs` (20 insertions, 66 deletions) and
is reviewed against the cumulative diff `67f855af^..cc752bcf`. The author,
Codex trailer, and `Step-Id: 01-01` remain mechanically correct, and the
second RED → GREEN → COMMIT cycle is recorded by DES.

Verification performed for this iteration:

- `cargo xtask lima run -- cargo check -p overdrive-control-plane --all-targets --features integration-tests`: **PASS**.
- `cargo xtask lima run -- cargo clippy -p overdrive-control-plane --all-targets --features integration-tests -- -D warnings`: **PASS**.
- `cargo xtask lima run -- cargo test -p overdrive-core --test acceptance netns_density_exec_gate -- --ignored --nocapture`: **6 passed**; the bodies remain ignored in source.
- `git diff --check 67f855af^ HEAD`: **PASS**.
- Source inspection confirms the remediation removes the production shared-network task spawn, `tokio::select!` classification, `fail_stop_or_pending`, and optional supervisor fallback. The only remaining supervisor task construction is in ignored source-local D8 tests; the production composition uses `task: None` in the approved RED scaffold.
- No mutation testing was run. No production or test code was modified by this review.

### Prior-finding remediation dispositions

#### F-01 — resolved

The remediation restores the exact non-optional private field at
`crates/overdrive-control-plane/src/lib.rs:1231`:
`shared_network_supervisor: SharedNetworkSupervisorHandle`. The public
`ServerHandle::shutdown_requested` is again a direct delegation at
`:1785-1790`, with no `Option` or pending compatibility branch. The private
supervisor method is explicitly the approved RED scaffold (`panic!("Not yet
implemented -- RED scaffold ...")`) at `:1266-1274`, and the later-step task
classification behavior is no longer present.

The production helper now creates only an unspawned scaffold owner with
`task: None` at `:2545-2552`; it does not submit a task or retain a second
owner. This is the bounded compile-time composition required by `01-01`,
while step `03-03` retains ownership of replacing the scaffold with the
behavioral supervisor.

**Disposition:** RESOLVED. The exact private shape is restored and no
out-of-step runtime behavior remains.

#### F-02 — resolved

The reversed `tokio::select!`, `Ok(Err(_))` source discard, and
`fail_stop_or_pending` implementation were deleted with the later-step body.
There is no production classification path left to mask a terminal join result
or discard a typed source. The previously failing ignored matrix is correctly
left pending behind the explicit RED scaffold; it is not activated by this
step.

**Disposition:** RESOLVED. The failed regression established the necessity of
removing the premature implementation; the remediation removes its entire
cause rather than retaining a partial later-step fix.

#### F-03 — resolved

The remediation deletes the production `tokio::spawn` and the parked
`task_shutdown`/request-owner composition. A boot error can therefore no
longer detach a shared-network task before a `ServerHandle` exists. The
retained scaffold's `shutdown()` remains owned by the `ServerHandle` paths and
has no task to await until the approved later step supplies one.

**Disposition:** RESOLVED. The detached async effect is absent from the
cumulative step diff.

### Iteration 2 design/API and test assessment

- The exact `ServerHandle` private owner field is now non-optional, matching
  the accepted D8 shape. Both public shutdown-request methods remain the only
  added delegation surface; no compatibility API or new error variant was
  introduced.
- The only production `shutdown_requested` body is the approved RED scaffold,
  which is appropriate because the retained classification/recovery behavior
  belongs to step `03-03`. The later-step ignored owner matrix remains
  reasoned-pending rather than being silently activated.
- The worker and core acceptance bodies retain their Contract Shape
  declarations and ignore markers. No assertion weakening, fixture theater,
  or scope expansion was introduced by remediation.
- The cumulative file-scope expansions remain the same tightly bounded
  compiler/API fallout assessed in iteration 1; the remediation adds no new
  files or public surface.

## Final verdict

# APPROVED

All iteration-1 blocking findings are resolved in
`cc752bcf0cd83707c06071dcc88b2b0bbfd88b37`. No unresolved findings remain for
roadmap step `01-01`.
