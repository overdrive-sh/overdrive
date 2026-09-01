# Architecture Review — DISTILL Expectation Boundary

## Current review status

| Field | Value |
|---|---|
| Current iteration | 4 |
| Current reviewed commit | `f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` |
| Current verdict | **APPROVED** |

## Iteration 1 — expectation-boundary restoration

## Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Reviewer | `nw-solution-architect-reviewer` |
| Review date | 2026-08-29 |
| Reviewed commit | `9558af759e049564b7149bea6459961979899e96` |
| Parent commit | `c33f0396edf86c1db888a4c36b751911258c48fb` |
| Review scope | Architecture boundary of the remediated DISTILL expectation, example bundle, scenario mappings, and downstream roadmap handoff |
| Verdict | **APPROVED** |

## Verdict summary

The targeted commit establishes the required test architecture: one public,
black-box E07 operator journey owns only the stakeholder-visible reply outcome,
while Rust tests retain all internal D7, Q7, failure, reclamation, and stop
obligations. The checked-in example uses a supported `[job]` + `[vm]` caller and
`[service]` + `[exec]` callee. Neither the expectation nor the example invents a
public protocol or product surface.

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

There are no approval-blocking findings.

## Scope and method

The review compared the immutable target commit with:

- the approved DESIGN D6, D7, and Q7/Q9 boundaries in
  `design/wave-decisions.md` and `feature-delta.md`;
- the architecture constraints in ADR-0088 and ADR-0089;
- the repository's expectation, integration-test, example, and public-surface
  rules;
- the DISTILL scenario ownership and contract-shape mappings; and
- the pending roadmap only to verify propagation of the corrected boundary.

The roadmap remains explicitly `pending` with `requires_regeneration: true`.
This review does not approve it for execution or supersede its required fresh
roadmap review.

## Architecture boundary assessment

| Boundary | Result | Evidence |
|---|---|---|
| One E07 public expectation | Pass | `verification/expectations/E07-guest-first-mesh-dial-born-captured/README.md:7` defines exactly one Exec Service and one VM Job and limits product driving to built `serve`, `deploy`, `workload describe`, and `job stop` commands. `verification/expectations/INDEX.md` lists E07 as the feature's sole expectation. |
| Black-box independence | Pass | The E07 runner imports or links no `overdrive-*` crate, invokes no Rust test binary or harness, and implements no kernel observer. Its pending stub only validates the checked-in fixture set; the README assigns the eventual run to the built default-feature product. |
| Example composition | Pass | `caller.toml` contains `[job]` + `[vm]`; `callee.toml` contains `[service]` + `[exec]`. There is no unsupported `[service]` + `[vm]` composition. The caller uses an ordinary TCP connection to `gti-e07-callee.svc.overdrive.local` and returns success only for the byte-exact callee reply. |
| No public-surface invention | Pass | The commit changes documentation, verification assets, and root examples only. It adds no production protocol, Beacon message, API/schema field, persistence shape, observation surface, crate, daemon, or dependency. The example uses existing accepted workload sections and existing product commands. |
| D7 ownership | Pass | `test-scenarios.md:93` keeps strict GETRULE/GETGEN framing, complete normalized program identity, notification/loss detection, capture integrity, exact packet/IPv4 `tot_len` accounting, TLS/kTLS, generation stability, and cleanup complements in S-GTI-02/S-GTI-03 and `P-GTI-D7-*` Rust tests. E07 expressly forbids inspecting or duplicating them. |
| Q7 and failure ownership | Pass | S-GTI-05, S-GTI-08a, and S-GTI-08b retain production failure-port, pre-READY, diagnostic, terminal-classification, cleanup, and post-READY exit obligations in Rust. No corresponding EDD observer remains. |
| D6 reclamation and stop ownership | Pass | S-GTI-06a/06b and S-GTI-12a/12b retain same-allocation Platform Reclamation, failed reinstall, exact-handle teardown, sibling preservation, and repeat-stop semantics in Rust. The public E07 cleanup command does not claim those internal proofs. |
| E08/E09 removal | Pass | The obsolete pending E08 and E09 expectation directories are deleted, the index exposes only E07, and the target tree contains no residual E08/E09 mapping. Their former internal contracts remain mapped to executable Rust obligations. |
| Roadmap handoff | Pass | Step 02-04 separates strict Rust D7 evidence from the sole built-product E07 outcome; steps 02-05 and 02-06 keep failure and reclamation/stop evidence Rust-only. The roadmap's pending/regeneration gate prevents accidental execution under superseded expectation ownership. |

## Detailed review

### E07 is a legitimate product-boundary witness

E07 proves one externally meaningful fact: a VM Job can address an Exec Service
by its service name, receive the expected reply, and reach the ordinary public
successful terminal result. The checked-in caller makes that result causally
useful by returning success only after exact response comparison. Therefore the
runner can observe the public result without decoding private lifecycle state
or recreating the implementation in shell or another helper.

The example is an operator-runnable product journey rather than a test-harness
surrogate. Its source and TOML are checked in at repository root, while the
runner is permitted only to compile/materialize those assets and drive the
built product. The helper is workload payload, not a duplicate product or
kernel observer.

### Internal guarantees remain at the Rust boundary

The correction does not weaken or erase the feature's internal obligations.
The DISTILL mapping retains separate executable Rust ownership for:

- strict nft generation/rule decoding, full rule identity, loss detection,
  exact counter and capture equality, original-destination delivery, and wire
  protection;
- pre-READY failure closure, exact exit-code classification, diagnostic
  precedence and bounds, cleanup totality, interruption, and replay;
- same-id boot-epoch reclamation and reinstall failure; and
- exact target-only stop, sibling preservation, idempotency, and cleanup
  complements.

That separation matches the production composition boundary: Rust integration
tests may compose `overdrive-*` crates in-process and inspect private mechanics,
whereas an expectation directly drives the built default-feature binary and
observes only operator-visible behavior. No expectation invokes `cargo test`, a
Rust test binary, or crate-linked observer, and no Rust test is assigned the
built-product evidence role.

### Roadmap quality checks at the reviewed boundary

The pending roadmap was not treated as executable, but its remediated boundary
was checked against the six architecture roadmap criteria:

| Check | Result | Assessment |
|---|---|---|
| External validity | Pass | E07 is assigned to the built product on qualified native metal; private claims stay with Rust tests. |
| Acceptance-criteria coupling | Pass | The E07 criterion states an observable reply-dependent outcome and does not prescribe a new internal API or protocol. The detailed Rust criteria trace ratified DESIGN contracts. |
| Step decomposition | Pass for reviewed slice | E07 and D7 share step 02-04 because they prove complementary public/internal cuts of the same flow; Q7 failure and D6 reclamation/stop remain separately owned by 02-05 and 02-06. |
| Implementation code in roadmap | Pass | The roadmap specifies behavioral contracts, ownership, and verification surfaces; it contains no pasted implementation. |
| Concision | Not a readiness verdict | The roadmap is a retained remediation handoff and explicitly requires regeneration. Its stale whole-document readiness is outside this targeted DISTILL review. |
| Unit-test boundary | Pass | Component/integration obligations do not substitute for E07, and E07 does not absorb private component or kernel assertions. |

## Verification performed

The following read-only checks passed against the target commit:

| Check | Result |
|---|---|
| Targeted diff whitespace validation (`git diff --check`) | Pass |
| E07 runner shell syntax (`bash -n`) | Pass |
| Roadmap JSON parse (`jq empty`) | Pass |
| Caller/callee TOML parse | Pass; exact section sets are `job/resources/vm` and `exec/listener/resources/service` |
| Checked-in helper compilation with host `rustc --edition 2021` | Pass (syntax/tooling check only; not native-metal evidence) |
| E07 runner executable mode | Pass (`100755`) |
| Target-tree search for stale E08/E09 references | Pass; none remain in the reviewed product/DISTILL/verification mapping |

No mutation testing or feature runtime testing was performed. E07 correctly
remains `pending`; this architecture approval does not constitute expectation
evidence.

## Final disposition

**APPROVED.** The commit now respects the repository's three evidence layers:
root examples express the user journey, E07 drives the built product and
observes the public reply-dependent outcome, and Rust tests prove internal
protocol, kernel, lifecycle, failure, reclamation, and cleanup guarantees.

The reviewer wrote only this review artifact. All pre-existing tracked and
untracked workspace changes were preserved and left untouched.

## Iteration 2 — runnable-journey remediation

### Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Reviewer | `nw-solution-architect-reviewer` |
| Review date | 2026-08-29 |
| Reviewed commit | `f064e0566611b1b8c4da7775fc169b929bccbca3` |
| Parent commit | `9558af759e049564b7149bea6459961979899e96` |
| Review scope | Iteration-2 architecture and feasibility review of the sole E07 expectation, its operator-runnable example, verification substrate handoff, and preserved Rust-only obligations |
| Verdict | **NEEDS_REVISION** |

### Verdict summary

The remediation preserves the central topology and ownership decisions from
iteration 1: this feature still has exactly one expectation, E07; its sole
checked-in journey still deploys one supported `[job]` + `[vm]` caller and one
supported `[service]` + `[exec]` callee; E08/E09 remain absent from the
expectation catalogue; and D7, Q7/failure, reclamation, stop, and protocol
internals remain mapped to Rust tests.

The new runnable example nevertheless introduces one high-severity boundary
regression and two medium-severity feasibility/safety defects. The E07
contract and operator script now inspect, assert, and destructively repair
private product cleanup state even though the same artifacts reserve private
cleanup for Rust. In addition, the credential lifecycle does not isolate the
production session keyring, and the preparation cleanup trap is armed after
materialization has already begun. These findings prevent approval.

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 1 |
| Medium | 2 |
| Low | 1 |

### Prior-disposition audit

| Iteration-1 disposition | Iteration-2 state | Evidence |
|---|---|---|
| One feature expectation, E07 | Closed | The target tree contains one feature expectation directory, `E07-vm-job-calls-exec-service`, and one corresponding index/roadmap mapping. The former E07 slug is removed rather than duplicated. |
| Supported example composition | Closed | `caller.toml` parses as exactly `job/resources/vm`; `callee.toml` parses as `exec/health_check/listener/resources/service`, with `replicas = 1`. No `[service]` + `[vm]` composition exists. |
| Black-box built-product driving surface | Partially regressed | The behavioral path remains built `serve`/`deploy`/`describe`/`job stop`, but the new operator script adds private runtime-resource observation and destructive cleanup. See ARCH-E07-I2-01. |
| No public protocol/product-surface invention | Closed | The commit changes documentation, example/verification assets, and the generic evidence harness only. The explicit-empty startup policy is an existing supported TOML surface. No Beacon, REST/OpenAPI, persistence, observation, crate, daemon, or dependency surface is added. |
| D7 remains Rust-owned | Closed | S-GTI-02/S-GTI-03 and `P-GTI-D7-*` retain netlink framing, normalized program identity, counters, capture, TLS/kTLS, generation, loss, and wire guarantees. E07 contains no such observer. |
| Q7/failure and D6 reclamation/stop remain Rust-owned | Closed except private-cleanup leakage | Scenario mappings still assign these contracts to Rust. E07 does not claim failure or reclamation behavior, but its runner now duplicates a private cleanup complement. |
| E08/E09 removed as expectations | Closed | Neither identifier has an active expectation directory, catalogue row, DISTILL expectation mapping, nor roadmap expectation mapping in the target tree. Their behavioral obligations remain in Rust scenarios and properties. |
| Pending roadmap gate | Closed | `roadmap.json` remains `validation.status: pending` with `requires_regeneration: true`; the renamed E07 handoff does not authorize execution. |

### Findings

#### ARCH-E07-I2-01 — High — E07 crosses the public expectation boundary into private cleanup

The expectation contract says E07 must not inspect private cleanup and that
Rust owns those guarantees (`verification/expectations/E07-vm-job-calls-exec-service/README.md:61-65`).
The same contract nevertheless makes evidence cleanup fail on VM/cgroup
residue (`README.md:54-59`). The shared operator script then:

- enumerates product-private `ovd-*` links and network namespaces and scans
  `/proc` for allocation processes (`run-example.sh:66-98`);
- kills newly observed processes and directly deletes product-created network
  namespaces and links (`run-example.sh:100-123`);
- recursively removes private VM run directories and writes `cgroup.kill`
  before directly removing allocation cgroups (`run-example.sh:125-154`); and
- makes the successful invocation fail when those private resources remain
  (`run-example.sh:174-199`).

These are not marker-owned example fixtures. They are production lifecycle
resources whose ownership, cleanup complements, target filtering, sibling
preservation, and idempotency are explicitly assigned to Rust integration and
component tests. Direct deletion can also mask the product cleanup defect that
the Rust layer is responsible for detecting. The distinction between an
operator example and an expectation runner does not repair the boundary: the
roadmap makes the shared preparation/run entry points part of E07's handoff,
and the checked-in operator example's own exit status includes these private
assertions.

Required remediation: keep E07's evidence/result boundary limited to public
deploy/describe/stop outcomes, the exact serve PID it started, and its
marker-owned preparation tree. Remove product-private run-dir, cgroup,
namespace, link, and process assertions/deletion from the example/expectation
contract. If shared-metal emergency recovery is required, place it in the
generic metal steward as explicitly non-E07 infrastructure handling that
invalidates the run; do not make it a feature expectation or duplicate the
Rust cleanup oracle.

#### ARCH-E07-I2-02 — Medium — The claimed per-run KEK is not isolated from the production session keyring

`prepare.sh` writes a fresh 32-byte credential (`prepare.sh:214-215`), but
`run-example.sh` starts `overdrive serve` in the caller's existing session
keyring (`run-example.sh:281-283`). Production
`SystemdCredsKeyring::resolve` searches that keyring first and consults
`CREDENTIALS_DIRECTORY` only on a miss. The repository's O04 runner documents
the consequence explicitly: every boot needs a fresh `keyctl session -`
because a cached KEK otherwise wins over the supplied credential.

The attempted compensation in `prepare.sh cleanup` is neither isolated nor
fail-closed: it purges the stable production description from the current
session only when the `keyctl` CLI happens to exist, suppresses every purge
failure, and cannot distinguish an E07-created key from a pre-existing key
(`prepare.sh:257-259`). Therefore the fresh credential may be ignored, while
the cleanup can delete state it never established. This contradicts the
documented per-run and delta-scoped lifecycle.

Required remediation: create and require a fresh session keyring for the
bounded serve lifecycle before production resolves the KEK, ensure the exact
serve process and signal handling remain owned, and let that isolated keyring
die with the run. Remove the best-effort purge of the shared stable
description; no cleanup path should delete an unproven pre-existing key.

#### ARCH-E07-I2-03 — Medium — Preparation traps are installed after materialization starts

The example promises that traps are installed before the first
materialization, but `prepare.sh` creates `$OUTPUT_ROOT` and writes its marker
at lines 184-185, then arms EXIT/signal traps at lines 186-187. An error or
signal in that interval can leave a partial fixed tree. If marker creation
fails after the directory is created, later `cleanup` refuses the unmarked
tree, permanently blocking the next prepared run without manual intervention.

Required remediation: arm cleanup before creating the fixed output and track
process-local creation ownership independently of the durable marker so every
partial-create phase is safely removable without weakening refusal of a
pre-existing unowned path. Preserve the bounded unmount/loop-detach behavior.

#### ARCH-E07-I2-04 — Low — Verification layout documentation still says every runner executes in Lima

The substrate model now correctly supports checked-in `native-metal`
metadata, but the directory-layout description at `verification/README.md:110`
still describes `run-expectation.sh` as running `runner.sh` in Lima. Update the
line to say it executes on the declared substrate so the catalogue has one
coherent architecture description.

### Architecture boundary matrix

| Boundary | Result | Assessment |
|---|---|---|
| Exactly one feature expectation | Pass | One E07 catalogue directory and mapping; no E08/E09 expectation survives. |
| Exactly one caller/callee journey | Pass | One `[job]` + `[vm]` caller addresses one replica of a `[service]` + `[exec]` callee. |
| Public reply-dependent outcome | Pass | Caller zero exit remains causally dependent on the exact response; public `describe` supplies `Terminated` plus `Verdict: Succeeded`. |
| No test-harness/crate substitution | Pass | The E07 runner invokes no `cargo test`, nextest, Rust test binary, or `overdrive-*` crate; the operator script builds only the default product binary and static workload helpers. |
| No duplicate D7 observer | Pass | No nft, netlink, capture, counter, TLS/kTLS, generation, or original-destination observer exists in E07. |
| Private lifecycle/cleanup separation | Fail | The new script asserts and repairs product-private process, namespace, link, run-dir, and cgroup state. |
| Public surface stability | Pass | Existing workload TOML and CLI surfaces are reused; `[service]` + `[vm]` remains absent. |
| Runnable preparation isolation | Fail | Session-keyring ownership and the pre-trap creation interval prevent the documented per-run, delta-scoped lifecycle from being reliable. |
| Rust-only E08/E09 obligations | Pass | Failure, reclamation, stop, sibling, nft/FIB, and replay contracts remain executable Rust obligations with no EDD mapping. |

### Roadmap boundary check

The roadmap remains deliberately pending, so this is not a readiness approval.
At the reviewed boundary, external validity, E07/Rust separation, step
decomposition, behavioral acceptance-criterion shape, absence of pasted
implementation, and unit/integration ownership remain sound. The renamed E07
paths and new preparation/run files are propagated consistently. The roadmap
must not be regenerated as approved until ARCH-E07-I2-01 through
ARCH-E07-I2-03 are closed and independently re-reviewed.

### Verification performed

The following read-only or `/tmp`-scoped checks were performed against the
target commit:

| Check | Result |
|---|---|
| Targeted diff whitespace (`git diff --check`) | Pass |
| Shell syntax for both example scripts, E07 runner, and catalogue harness | Pass |
| ShellCheck for both example scripts, E07 runner, and catalogue harness | Pass |
| `prepare.sh check-source` | Pass |
| Pending E07 runner exit contract | Pass; exact exit code `75` |
| Roadmap JSON parse | Pass |
| Caller/callee TOML parse | Pass; supported section sets confirmed |
| Caller/callee host compilation with `rustc --edition=2024 -D warnings` | Pass; compile-only feasibility signal, not native-metal evidence |
| Target-tree expectation inventory and stale E07/E08/E09 search | Pass; exactly one feature E07 mapping and no E08/E09 expectation |
| D7/test-harness leakage search | Pass for D7 and crate/test invocation; private-cleanup leakage was found by direct script review |

No feature runtime, native-metal evidence, Rust test suite, or mutation testing
was run. E07 correctly remains `pending`, and the harness now fails closed on
the pending stub rather than presenting it as successful execution.

### Iteration-2 disposition

**NEEDS_REVISION.** The sole-expectation topology, supported workload
composition, public reply proof, and Rust ownership of D7/E08/E09 behavior are
correct. Approval is withheld until the public/private cleanup boundary, KEK
session isolation, and pre-materialization trap lifecycle are corrected. The
low-severity substrate-documentation inconsistency should be fixed in the same
remediation.

The reviewer modified only this review artifact. All pre-existing tracked and
untracked workspace changes were preserved and left untouched.
## Iteration 3 — runtime-boundary remediation re-review

### Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Reviewer | `nw-solution-architect-reviewer` |
| Review date | 2026-08-29 |
| Reviewed commit | `5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Parent commit | `f064e0566611b1b8c4da7775fc169b929bccbca3` |
| Cumulative range | `9558af759e049564b7149bea6459961979899e96..5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Review scope | Iteration-3 disposition of all four iteration-2 findings, cumulative E07 architecture, Service/Job parser feasibility, and generic harness/E06 compatibility |
| Verdict | **NEEDS_REVISION** |

### Verdict summary

Commit `5279a561` closes all four findings raised in iteration 2. The E07
operator journey no longer observes or repairs product-private allocation
resources; the production KEK is resolved inside a fresh anonymous session
keyring without ambient purge; preparation traps and process-local ownership
are established before the first fixed-tree mutation; and the verification
catalogue now describes generic declared-substrate execution consistently.

The cumulative target also preserves the non-negotiable architecture: this
feature has exactly one expectation, E07; the sole example contains one
supported `[job]` + `[vm]` caller and one supported `[service]` + `[exec]`
callee; E08/E09 remain Rust-only; and no D7/protocol/private-lifecycle observer
has migrated into verification.

One new medium-severity cleanup-totality defect remains. The isolated-session
wrapper is reaped with an unbounded shell `wait`, including a failure/signal
path where the serve PID has not yet been recorded and the wrapper is never
signalled. That contradicts the example's finite cleanup contract and can hold
the host-global metal lease indefinitely. Approval remains withheld until that
wait/termination path is bounded.

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 0 |
| Medium | 1 |
| Low | 0 |

### Iteration-2 finding dispositions

| Finding | Disposition | Evidence |
|---|---|---|
| ARCH-E07-I2-01 — private cleanup crossed the E07 boundary | **Closed** | `run-example.sh` removes the process/netns/link/run-dir/cgroup snapshot, assertion, and destructive-repair code. Cleanup now uses required public stop calls, the exact started serve process, and the token-marked preparation tree only. E07 and DISTILL explicitly prohibit private cleanup inspection or repair. |
| ARCH-E07-I2-02 — KEK not isolated from the production session keyring | **Closed** | `run-example.sh:263-294` requires `keyctl session -`, verifies the new session is accessible and initially lacks the stable production KEK description, then `exec`s built `serve` with `CREDENTIALS_DIRECTORY`. `prepare.sh` no longer purges any ambient key. The session is scoped to the serve lifecycle. |
| ARCH-E07-I2-03 — preparation traps armed after mutation | **Closed** | `prepare.sh:213-227` arms EXIT/signal traps and records process-local ownership before creating `$OUTPUT_ROOT` or its durable marker. Partial output removal is separated from durable marker-based cleanup, preserving refusal of unowned persisted paths. |
| ARCH-E07-I2-04 — generic verification docs remained Lima-only | **Closed** | `verification/README.md` now describes declared-substrate execution and documents the host-safe harness branch test. `verification/expectations/INDEX.md` likewise distinguishes Lima and native-metal runners. |

### New finding

#### ARCH-E07-I3-01 — Medium — The isolated-session wrapper has an unbounded cleanup wait

`terminate_serve` bounds the ordinary serve TERM polling to five seconds and
may issue KILL, but it then calls bare `wait "$SESSION_WRAPPER_PID"` with no
deadline (`examples/guest-stack-transparent-mtls-intercept/run-example.sh:63-85`).
There is also a concrete earlier failure/signal branch where
`SESSION_WRAPPER_PID` has been captured but `SERVE_PID_FILE` has not yet been
written (`run-example.sh:295-303`). In that state `SERVE_PID` is empty,
`serve_pid_is_live` returns false, no process is signalled, and cleanup blocks
indefinitely waiting for the still-live keyctl/session wrapper.

Even after a recorded PID, a wrapper or killed child stuck before reap leaves
the same unbounded wait. This violates the documented guarantee that every
serve and cleanup wait has a finite deadline, can prevent marker-owned fixture
cleanup, and can retain the canonical host-global metal lease indefinitely.

Required remediation: give the exact `$!` session-wrapper process its own
bounded termination/reap lifecycle independent of whether the serve PID file
was created. On signal or early failure, signal only that owned wrapper;
observe exit through a finite TERM/KILL deadline; and invoke shell `wait` only
after the owned child is known to have exited. Preserve the exact-executable
guard before signalling a recorded serve PID.

### Cumulative architecture assessment

| Boundary | Result | Assessment |
|---|---|---|
| Exactly one feature expectation | Pass | The target tree contains one feature E07 directory, catalogue row, DISTILL mapping, and roadmap mapping. The superseded E07 slug and E08/E09 expectation directories remain absent. |
| Exactly one supported example topology | Pass | TOML parses to one `job/resources/vm` caller and one `service/exec/listener/resources/health_check` callee with `replicas = 1`; `[service]` + `[vm]` remains rejected by the production parser. |
| Service parser architecture | Pass | The existing explicit `health_check.startup = []` branch suppresses inferred startup probes without adding a new surface. The Service observer reads the real `Alloc / State` table and exact `Replicas (desired/running): 1/1` line. |
| Job parser architecture | Pass | `[job]` + `[vm]` remains an accepted existing driver dispatch. The Job observer separately reads the `Attempt / State` table and requires `Terminated` plus the public `Verdict: Succeeded`; cross-kind parser self-checks reject the other table shape. |
| Public black-box outcome | Pass | E07 uses built default-feature `serve`, `deploy`, `workload describe`, and `job stop`; caller success remains causally dependent on the byte-distinct exact reply. |
| Private cleanup boundary | Pass | E07 no longer inspects or repairs allocation processes, run directories, cgroups, namespaces, links, capture state, nft/FIB state, or lifecycle actions. |
| D7 and E08/E09 Rust ownership | Pass | Netlink framing, normalized programs, capture/counters, TLS/kTLS, generation/loss, failure, diagnostics, reclamation, stop/idempotency, sibling preservation, and cleanup complements remain Rust-only obligations. |
| No crate/test-harness substitution | Pass | E07 invokes no `cargo test`, nextest, Rust test binary, or `overdrive-*` crate; static Rust files are workload payloads and the only cargo build is the default product binary. |
| Public product-surface stability | Pass | The cumulative commits add no Beacon, API/schema, persistence, observation, daemon, crate, or dependency surface. Existing workload TOML and CLI commands are reused. |
| Owned preparation lifecycle | Pass | Token-specific marker validation, refusal of pre-existing output, pre-mutation traps, bounded mount/loop operations, and exact marker-tree cleanup close the previous ownership gap. |
| Isolated KEK lifecycle | Pass except teardown totality | Fresh-session absence and credential delivery are architecturally correct and ambient keys are untouched; ARCH-E07-I3-01 prevents the wrapper lifecycle from being fully bounded. |
| Roadmap execution gate | Pass | Roadmap validation remains `pending` with `requires_regeneration: true`; no E07 evidence is claimed. |

### Harness and E06 compatibility

The generic harness change is backward-compatible at the reviewed boundary:

- expectations without metadata still default to `lima`;
- checked-in `native-metal` metadata records
  `execution_substrate: native-metal` and `executed_in_lima: false`;
- runner exit 0 alone maps to `succeeded`; exit 75, missing runners, invalid
  substrates, and other failures return nonzero;
- the new host-safe branch suite executes all of those cases successfully;
- E06 now has the missing `native-metal` declaration, while its runner logic
  changed only explanatory text and substrate-record narration; and
- E06's already-reviewed historical evidence files are untouched, with their
  legacy field explicitly documented rather than rewritten.

No E06 behavior, evidence verdict, production command, or kernel observer is
reassigned to E07. The feature-scoped single-expectation constraint means one
expectation for `guest-stack-transparent-mtls-intercept`, not removal or
renumbering of unrelated catalogue entries such as E06.

### Verification performed

The following read-only or `/tmp`-scoped checks were performed against the
cumulative target:

| Check | Result |
|---|---|
| Cumulative targeted diff whitespace (`9558af75..5279a561`) | Pass |
| Shell syntax for example preparation/run scripts, E07/E06 runners, and generic harness scripts | Pass |
| ShellCheck for the same scripts | Pass with only E06's pre-existing dynamic-source `SC1091` excluded |
| Host-safe generic harness branch suite | Pass |
| `prepare.sh check-source` and `run-example.sh check-source` | Pass, including Service/Job parser-shape self-checks |
| Pending E07 runner exit contract | Pass; exact exit code `75` |
| Roadmap JSON parse | Pass |
| Caller/callee TOML parse | Pass; supported section sets and single-replica Service confirmed |
| Caller/callee host compilation with `rustc --edition=2024 -D warnings` | Pass; compile-only feasibility signal, not native-metal evidence |
| Target-tree E07/E08/E09 inventory | Pass; one feature E07 and no E08/E09 expectation |
| E07 private/D7 observer and crate/test invocation search | Pass; no active implementation leakage found |
| Production parser/renderer inspection | Pass; existing `[job]+[vm]`, `[service]+[exec]`, explicit-empty startup, Service table, and Job table contracts align with the example |
| E06 compatibility diff | Pass; declaration/documentation only, historical evidence unchanged |

No mutation testing, native-metal execution, feature runtime, or captured E07
evidence was performed. E07 correctly remains `pending`.

### Iteration-3 disposition

**NEEDS_REVISION.** All four iteration-2 findings are closed, and the core
single-E07/Rust-only architecture is now correct. One medium-severity bounded
cleanup defect remains in the fresh-session wrapper lifecycle. Re-review can
be limited to that remediation and its finite early-failure/signal behavior.

The reviewer modified only this review artifact. All pre-existing tracked and
untracked workspace changes were preserved and left untouched.

## Iteration 4 — bounded launch-lifecycle remediation re-review

### Review metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Reviewer | `nw-solution-architect-reviewer` |
| Review date | 2026-08-29 |
| Reviewed commit | `f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` |
| Parent commit | `5279a561fcff74f61e8329f07fb6a72af0abe051` |
| Cumulative range | `9558af759e049564b7149bea6459961979899e96..f908e0cf7a07fdf7f90f22cfdbcd6b23edc237ab` |
| Review scope | Iteration-4 disposition of ARCH-E07-I3-01, wrapper-to-serve handoff safety, bounded signalling/reaping, the host-safe lifecycle fault suite, the `keyctl` versus Rust-keyring launcher boundary, and cumulative E07/E08/E09 ownership |
| Verdict | **APPROVED** |

### Verdict summary

Commit `f908e0cf` closes ARCH-E07-I3-01. The E07 launch now has a bounded,
identity-checked ownership protocol from the direct `setsid` child through the
fresh-keyring launcher and final product `exec`. Cleanup can address the exact
direct child before group formation, the proven private process group after
formation, and the exact published serve identity after handoff. TERM and KILL
polling are finite, and shell `wait` is reachable only after process-table
absence, zombie state, or observed replacement proves that the original direct
child cannot still block.

The cumulative target preserves exactly one feature expectation, E07. E08 and
E09 remain absent as expectations, and their internal failure, reclamation,
stop, kernel, and cleanup guarantees remain Rust-only. The new lifecycle
library and host-safe fault test do not become a second product implementation
or evidence runner.

| Severity | Count |
|---|---:|
| Blocker | 0 |
| High | 0 |
| Medium | 0 |
| Low | 0 |

There are no approval-blocking findings.

### Iteration-3 finding disposition

| Finding | Disposition | Evidence |
|---|---|---|
| ARCH-E07-I3-01 — isolated-session wrapper had an unbounded cleanup wait | **Closed** | `run-example.sh:201-243` records the direct child before completing launch, proves its private group, acknowledges ownership, and bounds both readiness and serve-PID handshakes. `session-lifecycle.sh:106-197` signals only the start-time-matched child or adopted private group, polls finite TERM/KILL windows, and calls `wait` only after exit proof. `test-e07-session-lifecycle.sh:90-258` exercises pre-group, pre-ready, pre-PID, post-PID/pre-exec, handshake-timeout, normal TERM, TERM-to-KILL, reap, and unrelated-sentinel cases. |

### Launch ownership and teardown assessment

| Phase | Ownership proof | Failure or signal disposition |
|---|---|---|
| Background-launch handoff | `run-example.sh:201-223` temporarily records signal intent, captures `$!`, and binds it to the kernel start time before restoring exit-on-signal behavior. | An observed signal returns 130 after the direct child is addressable; EXIT cleanup then owns termination. If the child exits before capture, it has not passed the descendant gate. |
| Before private-group adoption | `session-wrapper.sh:22-54` proves `pgid == pid` and uses a builtin-only acknowledgement gate. No keyctl, shell child, or product process exists yet. | Cleanup verifies the direct PID/start-time pair and signals that child only. The wrapper's own acknowledgement deadline is five seconds. |
| Private-group acknowledgement | `session-lifecycle.sh:95-104,213-230` independently proves the live `setsid` leader, records the group, and atomically writes a token/PID/PGID/start-time acknowledgement. | From this point, all later descendants inherit a private group that the parent already owns. Invalid or missing handshakes fail within fixed polling counts. |
| Fresh-keyring and serve handoff | `session-wrapper.sh:56-102` publishes token-bound readiness before `keyctl`, then publishes the pre-exec process PID/PGID/start time atomically. `run-example.sh:187-199` revalidates that record against the owned group. | Failure in `keyctl`, the child shell, PID publication, or product `exec` remains inside the already-owned group and is handled by the same bounded teardown. |
| Running product | `run-example.sh:67-85` requires both unchanged PID/start/PGID and the exact built-product executable before treating the published identity as live `serve`. | Public workload stops remain bounded and run first. Launch-unit teardown then sends TERM and, if needed, KILL to the private group. |
| Reap and completion | `session-lifecycle.sh:106-129` distinguishes a live entry from zombie, absence, and PID replacement; Bash `wait` targets only its original direct-child job record. | Default TERM and KILL windows are each at most 50 polls of 0.1 seconds. Failure to disappear is reported rather than converted into success, so cleanup cannot retain the metal lease through an unbounded wait. |

The ownership order closes the early-failure hole from iteration 3. In
particular, the wrapper cannot create a descendant until the parent has proven
and acknowledged the private group. Once descendants are possible, negative
PGID signalling is restricted to that already-proven group. The published
serve record adds the per-process start time and group identity needed to
survive the keyctl/bash/serve `exec` chain without trusting a bare PID.

The fault suite is intentionally host-safe: it substitutes private temporary
process groups for keyctl, KVM, and the product, then drives the production
lifecycle helper through each ownership phase. Every case is wrapped in an
outer five-second timeout, checks that the direct child was reaped and its
group disappeared, and keeps an unrelated sentinel live. The TERM-to-KILL case
also proves that a TERM-resistant launch unit reaches escalation rather than an
unbounded wait. Repeating the complete suite 25 times produced 25 passes.

### `keyctl` versus Rust keyring API disposition

No finding is raised for `keyctl session -` in `session-wrapper.sh`.

The concern correctly identifies that the separate
[`keyutils` 0.4 `Keyring`](https://docs.rs/keyutils/0.4.0/keyutils/struct.Keyring.html)
API offers `join_anonymous_session`. The reviewed repository does not link that
crate, however. `Cargo.toml:83-90` and `Cargo.lock:1732-1740` pin
`linux-keyutils` 0.2.5, and its public `KeyRing` interface exposes attachment
to the current special session keyring but no public anonymous-session join.
Production `SystemdCredsKeyring` opens the already-current session keyring and
performs the actual KEK search, delivery, add, and read-back
(`crates/overdrive-host/src/ca/keyring.rs:205-209,363-393`).

Creating a fresh session keyring is therefore a process-launch context, not
product credential-resolution logic. A Rust call to
`join_anonymous_session` would still need a separate launcher process that
calls it and then `exec`s the built product; it cannot alter the parent shell's
session keyring. Replacing the canonical `keyctl session - <program>` boundary
would add a new compiled helper and a distinct Rust dependency without removing
the wrapper-to-product lifecycle problem.

The shell launcher also does not duplicate production KEK handling. Its
`keyctl describe` and `search` calls only prove the black-box precondition that
the newly joined session exists and initially lacks the production
description. It never reads, supplies, adds, replaces, or purges the key.
Production remains solely responsible for resolving the checked-in credential.
This is a justified external black-box launcher boundary, analogous to the
existing `setsid` and `timeout` process controls, and it introduces no
`overdrive-*` crate link or test-binary substitution.

### Cumulative architecture assessment

| Boundary | Result | Assessment |
|---|---|---|
| Exactly one feature expectation | Pass | The target tree contains one `E07-vm-job-calls-exec-service` directory and one feature mapping. No E08/E09 expectation directory or active mapping exists. |
| Supported example topology | Pass | TOML parses to `job/resources/vm` for the caller and `exec/health_check/listener/resources/service` with one replica for the callee. |
| Public black-box outcome | Pass | E07 still drives built default-feature `serve`, `deploy`, `workload describe`, and `job stop`, and accepts only the reply-dependent public successful Job result. |
| Launcher boundary | Pass | `setsid`, `keyctl session`, and the shell lifecycle helpers establish external process/keyring context; they do not reimplement Overdrive credential, protocol, kernel, or lifecycle behavior. |
| Private cleanup boundary | Pass | Cleanup uses public stop, its externally owned launch group, and marker-owned materialization only. It does not enumerate, assert, or repair product-private cgroups, namespaces, links, run directories, nft/FIB state, or capture state. |
| D7 and E08/E09 ownership | Pass | Netlink framing, normalized programs, capture/counters, TLS/kTLS, generation/loss, failures, diagnostics, reclamation, stop/idempotency, sibling preservation, and cleanup complements remain Rust-only. |
| No crate/test-harness substitution | Pass | The pending E07 runner imports no crate and invokes no Rust test binary. The host-safe lifecycle suite tests only the external launcher helper and is not expectation evidence. |
| Bounded lifecycle | Pass | Readiness, PID publication, TERM, KILL, public stop, preparation cleanup, and direct-child reap paths are finite; the previous bare live-child wait is gone. |
| Identity and unrelated-process safety | Pass | Direct-child start time, private PGID, launch token, serve start time, and exact executable are independently checked. Fault tests preserve an unrelated private-group sentinel in every case. |
| Roadmap execution gate | Pass | `validation.status` remains `pending` with `requires_regeneration: true`; E07 remains an exit-75 stub and no native evidence is claimed. |

### Verification performed

The following read-only or private-temporary checks were performed against the
reviewed target:

| Check | Result |
|---|---|
| Reviewed lifecycle files match target commit `f908e0cf` | Pass |
| Targeted and cumulative diff whitespace (`5279a561..f908e0cf`, `9558af75..f908e0cf`) | Pass |
| Shell syntax for preparation, operator, wrapper/lifecycle, E07 runner, and host-safe harness scripts | Pass |
| ShellCheck for the changed example/lifecycle scripts, E07 runner, and E07 lifecycle harness | Pass |
| Host-safe E07 lifecycle fault suite | Pass; 25 complete repetitions, including all seven fault modes |
| Generic expectation-harness branch suite | Pass |
| `prepare.sh check-source` and `run-example.sh check-source` | Pass, including Service/Job parser self-checks |
| Pending E07 runner contract with harness-provided `REPO_ROOT` | Pass; exact exit code `75` |
| Roadmap JSON parse and execution gate | Pass; `pending`, `requires_regeneration: true` |
| Caller/callee TOML parse | Pass; exact supported section sets and one Service replica confirmed |
| Target-tree E07/E08/E09 inventory | Pass; exactly one feature E07 and no E08/E09 expectation |
| E07 private/D7 observer and crate/test invocation search | Pass; no active implementation leakage |
| Keyring dependency and API inspection | Pass; target links `linux-keyutils` 0.2.5, not `keyutils` 0.4, and product versus launcher responsibilities remain separate |

No native-metal execution, feature runtime, mutation testing, or E07 evidence
capture was performed. E07 correctly remains `pending`.

### Iteration-4 disposition

**APPROVED.** ARCH-E07-I3-01 is closed. The fresh-session launch has finite,
identity-safe cleanup across the early wrapper, private-group, keyctl, pre-exec,
and running-serve phases; host-safe faults confirm reaping, TERM/KILL
escalation, and unrelated-process preservation. `keyctl session -` is a
justified external launcher boundary rather than a duplicate product helper.
Exactly one E07 remains, while E08/E09 and all private guarantees remain at the
Rust integration/component boundary.

The reviewer modified only this review artifact. All pre-existing tracked and
untracked workspace changes were preserved and left untouched.
