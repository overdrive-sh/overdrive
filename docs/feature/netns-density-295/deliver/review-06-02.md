# DELIVER Review — Step 06-02

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `06-02` — Owner allocation lifecycle |
| Reviewer | `nw-software-crafter-reviewer` |
| Iteration | 1 |
| Commits reviewed | `8ecb7936ce7ba328f3ca05b4e20019cef4bc2015`, `d8b5b4f8f317d5c665566720bd045719c97b7e2e` |
| Verdict history | Iteration 1: **NEEDS_REVISION**; latest: **APPROVED** (Iteration 2) |

## Scope and contract sources

Reviewed roadmap step 06-02 against the accepted R4/R21/R22 allocation and
host-side MAC contracts, the owner provision/read-back/teardown/audit
contracts, M2/H1, S-ND295-11/12/50/72, and E23. The applicable files are
`deliver/roadmap.json`, `feature-delta.md`, `distill/test-scenarios.md`,
`distill/red-classification.md`, and `.context/distill-fixture-06-02.md`.

The implementation delta is limited to the private host owner, its private
source-local test adapter, endpoint-map inventory, the existing netlink thread
bridge, the two startup integration bodies, and the DES log. These additions
serve the accepted owner lifecycle, unmanaged endpoint inventory, and E23
criteria. No new public, cross-crate, doc-hidden, or Sim API shape appears in
the two commits. The public owner signatures and error taxonomy are unchanged.

The provision path passes uid 0 to persistent TAP creation, attaches and reads
back the egress classifier and pin before the final TAP-down read-back, then
checks the observed host-side address against the held guest MACs and bridge
MAC. `TapHostMac` is formed only as an error fact; the recorded allocation
state has no host-side MAC field. The audit reads the bridge, guard, TCX
programs/maps/pins/endpoint inventory, and mask dump before attributing the
first per-allocation fault. Teardown uses the retained allocation state and
converges absent parts, skips TAP mutation/read-back when the TAP is already
absent, retains typed failures for retry, and checks the unrelated attachment
in its acceptance body.

The fixture repair preserves the owner's production classification path. The
host adapter reads the bridge identity and gateway-presence tuple, guard
inventory, TCX inventory results, and debug-mask dump; `FakeAttachmentKernel`
returns corresponding observed facts and typed observation failures. The owner
then applies the bridge, guard, node-component, and per-allocation checks. The
fixture does not return a precomputed `SharedGuestNetworkAudit` verdict. These
observations remain inside private owner/test-support boundaries.

## Contract Shape Compliance

The activated source-local allocation-owner bodies carry their existing
per-test `Outcome anchor` and `CONTRACT_SHAPE: bounded-change` rustdoc. The
E23 body carries `CONTRACT_SHAPE: unbounded-preservation`. The two activated
integration bodies carry `CONTRACT_SHAPE: bounded-change`. No source-local
pure-function property was added or transitioned, so the required exact
`/// CONTRACT_SHAPE: pure-function.` declaration is not applicable. The
changed test diff removes pending `#[ignore]` markers and adds private fixture
observation support; it does not weaken or replace an assertion or expected
value. Names in the changed tests do not match the banned test-name patterns.

The role skill's declaration-check CLI was not present at its expected
`~/.claude/lib/python/src/des/cli/check_contract_shape_declarations.py` path.
I checked the changed test declarations and names directly with `rg`.

## TDD, coverage, and evidence

The step activates 14 source-local owner tests and two integration bodies.
Counting the distinct observable outcomes as complete provision, incompatible
provision refusal and rollback, teardown convergence, typed teardown retry,
node-level audit precedence, per-allocation audit attribution, reserved versus
unreserved host-side MAC handling, the real-host udev audit, and E23 refusal
gives 9 behaviors and a 18-test budget. The 14 unit/acceptance tests are within
that budget. Integration bodies are assessed separately.

Test integrity is clean: no assertions were weakened, tests deleted, or
pending bodies activated outside this step. Tests exercise the owner through
its private driving boundary or the production server composition. The
integration bodies use the production owner in-process and do not launch the
Overdrive product binary or act as expectation runners. No mutation test was
run during this step; the unreserved-address contrast remains active for the
single final DELIVER-wave mutation gate.

The recorded DES history is `RED FAIL`, `RED PASS`, `GREEN PASS`, `COMMIT
PASS`; the earlier RED failure is retained. The separate baseline
reconstruction reported by the step crafter is the M15 evidence for semantic
S-ND295-50/S-ND295-72 audit RED while preserving the private fixture factoring.
It is not inferred from `.context/distill-fixture-06-02.md`: that artifact
records the original unrelated missing-bridge fixture failure and, after
repair, three bodies already green plus one semantic `TapDeleted` RED. It
explicitly does not claim M15 semantic RED for the three already-passing
bodies. The unreserved contrast was not claimed as RED or mutation-killed.

| Gate | Result | Evidence |
|---|---|---|
| G1 — activation discipline | PASS | Activated bodies are the step's DISTILL-authored S11/S12/S50/S72/E23 scope; S-ND295-50 condemned and S-ND295-72 activation/condemned bodies remain assigned to 06-04. |
| G2 — valid RED | PASS, per crafter report | Separate baseline reconstruction is reported for the semantic audit RED; the fixture artifact's missing-bridge failures are excluded. |
| G3 — assertion failure | BLOCKED | E23 permits success without the required thread refusal; see F1. |
| G4 — no internal domain mocks | PASS | The private fake sits at the owner I/O boundary; production owner classification runs unchanged. |
| G5 — business language | PASS | Bodies cite the accepted scenario and outcome anchor. |
| G6 — all green | PASS, reported | Targeted Lima, integration, and native runs are listed under Verification. |
| G7 — green before commit | PASS, reported | DES log records GREEN and COMMIT as PASS. |
| G8 — test budget | PASS | 9 behaviors; budget 18; 14 source-local unit/acceptance tests. |
| G9 — no test weakening | PASS | Diff inspection found activation and private fixture support changes, not reduced assertions. |
| External validity | PASS | Production composition startup bodies and real Lima/metal host audit body are included. |

**RPP scan:** L1 and L2 reviewed; no actionable smell is reported within this
step's approved sequential owner algorithm. No code-quality finding is used to
expand this step.

## Findings

### F1 — Blocker — E23 test does not require the refused-thread outcome

**Location:** `crates/overdrive-control-plane/src/guest_network.rs:11763`
(`Ok(_) => {}`), and `guest_network.rs:11765` (variant-only error match).

The accepted criterion requires the real `pids.max` refusal to return
`NetlinkError::Connect` carrying the spawn `io::Error`, with the process
continuing without panic or abort. The body successfully installs the cap via
`refuse_thread_creation()`, but `Ok(_) => {}` is an accepted result. The error
arm checks only the `Connect` variant and ignores its `source`. The test can
therefore pass without observing a refusal, or with a `Connect` originating
inside the closure's `Client::new()` call.

The production path at `crates/overdrive-netlink/src/runtime.rs:62` correctly
uses fallible `Builder::spawn_scoped` and maps its `io::Error` to
`NetlinkError::Connect` at line 63. This review does not claim a reproduced production
failure. The blocker is that the required E23 acceptance outcome is not
asserted by its regression body, so the reported passing run does not establish
that the refused-spawn branch returned its source. The test should fail if the
call returns `Ok`, then assert `Connect` and the spawn-refusal `io::Error` (or
an equivalent exact source assertion under the real `pids.max` stimulus).

**Reachability and ordering:** the test directly calls the production
`block_on_host_netlink` entry point after the helper moves the test process into
a scratch cgroup and writes `pids.max` to the current task count
(`crates/overdrive-testing/src/pids_max.rs:101` and `:104`). The production helper
attempts thread creation before calling the closure; a refusal is returned at
that boundary. There is no retry or detached task. Thus an accepted `Ok` is
observable at the actual entry point and bypasses the required refusal
assertion.

**Remediation disposition:** OPEN. Return this test to the original 06-02
crafter for a focused assertion correction and re-review. No production API or
error-shape change is needed.

### Unproven hypothesis — endpoint inventory uses different known-index sets

The production `observe_shared_tcx` adapter currently ignores its
`managed_ifindices` input, while `FakeAttachmentKernel::observe_shared_tcx`
uses the current set when classifying an endpoint key. The production
`GuestTcxInventoryIdentity` compares map keys with its retained receipt set.
That set is historical because insertion adds ifindices and teardown removes
the map key through a separate helper. A stale previously managed ifindex
could therefore be treated differently from a fresh unmanaged ifindex.

No failing regression through the production owner path was available to
establish that this state is reachable after the owner's teardown and
complement checks. Per repository rules this remains an unproven hypothesis,
not a finding or a remediation requirement.

## Verification

The following results were reported for this step; they were not rerun during
review.

| Command | Reported result |
|---|---|
| Lima owner acceptance: `cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(allocation_owner_acceptance) or test(shared_owner_link_address_kernel)' --no-fail-fast` | 16 passed |
| Lima E23 thread refusal: `cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(block_on_host_netlink_returns_connect_when_a_thread_is_refused)' --no-fail-fast` | 1 passed; F1 limits what this result proves |
| Lima startup integration: `cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(shared_guest_network_startup)' --no-fail-fast` | 9 passed |
| Metal udev audit: `cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(a_tap_host_address_is_judged_by_the_invariant_whatever_the_host_link_manager_wrote)' --no-fail-fast` | 1 passed |
| Lima workspace check | Passed |
| Lima workspace clippy with `-D warnings` | Passed |
| `cargo fmt --check`; `git diff --check` | Passed |

The production implementation, changed file scope, and commit metadata are
consistent with step 06-02. Commit `8ecb7936` retains Marcus as author, has
exactly `Co-Authored-By: Codex <codex@openai.com>`, contains `Step-Id: 06-02`,
and has no Claude/Anthropic attribution. The follow-up DES-log commit records
COMMIT PASS. The existing dirty `AGENTS.md` was preserved.

## Iteration 1 verdict

**NEEDS_REVISION.** The accepted public API shape, owner lifecycle behavior,
scope, test activation, and reported build/test gates are satisfactory. F1
blocks approval because the mandatory E23 regression does not require the
thread-spawn refusal or verify its source error. The endpoint-inventory note is
not an accepted finding without a failing production-path regression.

## Iteration 2 — Re-review of F1

### Metadata

| Field | Value |
|---|---|
| Reviewer | `nw-software-crafter-reviewer` |
| Iteration | 2 |
| Finding under review | F1 — E23 test did not require the refused-thread outcome |
| Commits reviewed | `0883fc401adba7a442f99e6e9dd9b01da8f588f2`, `740f7b59a10dd361cfbb3d2a711596b39cde7233` |
| Verdict | **APPROVED** |

### F1 disposition — CLOSED

The correction strengthens the existing DISTILL-authored E23 body. At
`crates/overdrive-control-plane/src/guest_network.rs:11769`, it requires the
result to be an error; line 11770 destructures only
`NetlinkError::Connect { source }`. The supplied closure increments an atomic
counter before constructing the netlink future (`:11759`), and the test
requires that counter to remain zero (`:11774`). It also requires
`source.raw_os_error() == Some(libc::EAGAIN)` (`:11779`) and
`source.kind() == WouldBlock` (`:11784`). A `Connect` from the closure, a worker
panic converted to another I/O error, a successful spawn, or a result that
enters the closure cannot satisfy these assertions.

The stimulus remains the real `overdrive_testing::pids_max::refuse_thread_creation`
fixture (`:11756`), which moves the test process into a scratch cgroup and sets
`pids.max` to its current task count. Production uses fallible
`Builder::spawn_scoped` at `crates/overdrive-netlink/src/runtime.rs:62` and
preserves the spawn error through `NetlinkError::connect` at line 63. The
reported Lima-root output was `NetlinkError::Connect` with OS errno 11,
`WouldBlock`, and `closure_entries=0`. This distinguishes the real spawn
refusal from errors produced inside the supplied closure and demonstrates that
the process reaches the assertions without panic or abort.

The correction changes only this existing test body and its E23 rustdoc, plus
the actual GREEN/COMMIT phase events. It adds no production behavior, API,
scenario, or other assertion changes. The original RED history remains intact;
no RED event was added for this evidence-only correction.

| Finding | Iteration 1 disposition | Iteration 2 disposition |
|---|---|---|
| F1 — E23 refused-thread result not required | OPEN | **CLOSED** — result, source errno/kind, and zero closure entries are asserted under real `pids.max` refusal. |

### Verification

The following results were reported for the correction:

| Verification | Reported result |
|---|---|
| Lima-root E23 refused-thread regression | 1/1 passed; `Connect`, errno 11 / `WouldBlock`, closure entries 0 |
| Lima step 06-02 owner acceptance | 16/16 passed |
| Lima `cargo check --workspace --all-targets --features integration-tests` | Passed |
| Lima workspace clippy with `-D warnings` | Passed |
| Changed-file formatting and `git diff --check` | Passed |

Both commits retain Marcus as author, each has exactly
`Co-Authored-By: Codex <codex@openai.com>` and `Step-Id: 06-02`, and neither
adds Claude or Anthropic attribution. The step log appends only the executed
GREEN PASS and COMMIT PASS events.

### Final verdict

**APPROVED.** F1 is closed with the required real spawn-refusal evidence and
source discrimination. No other step 06-02 finding remains open.
