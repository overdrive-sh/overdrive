# DELIVER review — STEP 14-02 native fault evidence

## Review metadata

| Field | Value |
| --- | --- |
| Feature / step | `netns-density-295` / `14-02` — Native fault evidence (P6-S14) |
| Review role | Fresh isolated `nw-software-crafter-reviewer`, DELIVER implementation review |
| Review iteration | 1 |
| Parent reviewed | `499b0345eba67b472a4a8f878fc269b7db5725e7` (reviewed through current `HEAD` `a11f0a9dcb007be06a4c3d4448e2b80e1de13c86`) |
| Implementation commit | `abfd83d38dbd84b259f8c074dbe16c344dfe8516` |
| DES follow-up commit | `a11f0a9dcb007be06a4c3d4448e2b80e1de13c86` |
| Final verdict | **APPROVED** |
| Reviewer-owned changes | This Markdown artifact only |

This review covers the complete step diff from the approved 14-01 parent to
`HEAD`, the exact step dispatch, the accepted feature delta and ADRs 0150,
0159, 0182, 0197, 0198–0201, 0204, 0209–0211 and the referenced approved
contracts. The native Markdown artifact convention takes precedence over the
installed review skill's legacy YAML-only output.

The user-dirty `AGENTS.md` was preserved. No source, test, model, design,
roadmap, DES log or host state was changed by this review. The historical
direct bare-metal receipts remain evidence of an execution-boundary violation
with zero acceptance credit; the review does not rerun or clean that state.

## Authority and review boundary

The accepted step requires five existing CLI integration bodies and one
existing worker body to pass through the approved GuestKit boundary:

1. `the_serve_owners_death_resets_every_far_end_and_the_next_boot_reclaims_the_vm`
   proves owner death, reset of both far ends, VMM survival until the next
   boot, NO-SURVIVE reclamation and re-placement on the reclaimed CID.
2. `a_guest_reset_ends_the_vmm_and_resets_every_far_end` proves guest PID-1
   reset, RESET-EXIT, host-side far-end reset and a restartable Service
   successor. The Service fixture is required because RESTART-SSOT finalizes
   a naturally crashed Job without a successor.
3. `a_killed_vmm_resets_the_far_end_of_its_flow` proves VMM SIGKILL and
   device-release reset of the far end.
4. `a_far_reader_that_stops_blocks_the_sender_and_the_flow_resumes_in_order`
   proves bounded backpressure, no corruption or reordering and resumption.
5. `closed_udp_sockets_release_their_slots_before_and_after_pairing` proves
   1,100 UDP send/close operations and a later round trip without slot
   exhaustion.
6. `leg_f_repair_rebinds_transparent_listener` proves real EMFILE failure of
   leg F, a still-LISTEN old socket, release-before-rebind repair, a new
   inode, a closed old descriptor and a second TPROXY-captured connection
   accepted and closed fail-closed.

The implementation notes require exact existing acceptance bodies, no new
production fault hook, no test-only VM driver seam, no public API invention,
and no mutation testing in this step. The accepted D25 contract requires all
slots of a released application socket to be released in discard,
disconnect, return order; K-B2 requires a lost release report to be found by
the level-triggered audit. RESET-EXIT and NO-SURVIVE are lifecycle contracts,
and D19-LINKS expressly excludes runtime link-detach injection.

The independently persisted DISTILL support review
`review-distill-14-02-guest-support.md` records ten iterations. Its final
support verdict is approved, closes worker findings F-01 through F-04 and UDP
assertion-order finding F-05, and approves the existing exact GuestKit worker
and CLI identities. That upstream artifact is evidence for support scope and
test-boundary decisions; this review independently checks the cumulative
implementation and current receipts.

## Commit, scope and DES integrity

Both commits preserve the Marcus author and carry exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer and one `Step-Id: 14-02`
trailer. No Claude, Anthropic or generated attribution appears. The follow-up
commit only persists the already-executed COMMIT event in the DES log.

The cumulative step diff contains the expected production, acceptance,
GuestKit support, Sim/QPROJ projection, formal-record and scenario-catalogue
files. It adds no production hook, configuration flag, public type, trait,
method, enum variant, wire field, port, queue, generation, protocol message,
or test identity. The `git diff --check` gate is clean for the changed source
and executable test files.

The DES log records the actual three-phase sequence:

| Phase | Result | UTC timestamp |
| --- | --- | --- |
| RED | FAIL | 2026-11-01 01:24:20 |
| RED | FAIL | 2026-11-01 02:18:29 |
| RED | FAIL | 2026-11-01 03:27:52 |
| GREEN | PASS | 2026-11-01 08:08:53 |
| COMMIT | PASS | 2026-11-01 08:13:50 |

`des-verify-integrity` reports no missing or incomplete phase for 14-02. Its
repository-wide output reports only unrelated historical 04-02–04-04 gaps and
future 14-03–14-05 gaps; those entries are not evidence for this step and were
not changed.

The meaningful RED evidence is preserved. The guest-reset Job attempt reached
the real RESET-EXIT failure but correctly produced no successor under
RESTART-SSOT; the bounded fixture correction changed only that existing
scenario to a restartable Service. The UDP native RED reached the production
guest and failed at send 1,042/1,100 with `EPERM`, ending the allocation
`Failed` instead of `Terminated`. The worker RED reached real `EMFILE` and
then exposed the fixture's incomplete shared-state setup. These are semantic
or fixture-contract failures, not fabricated compilation failures. The later
GREEN receipts close the corrected paths.

## Contract and implementation assessment

### UDP ownership, lifetime and batch semantics

`KernelGuestOwnerIo::drain_user_events` now drains the complete
`USER_EVENTS` batch before handling it (`crates/overdrive-init/src/guest_flow/kernel_io.rs:391-412`).
The private `release_unpublished_slots` helper scans the existing `SLOT_META`
array once for the batch, indexes claims by exact application cookie, and
constructs the existing slot address from the token and slot index
(`kernel_io.rs:414-452`). It treats a resident claim as prepublication only
when neither `connected_slots` nor `slot_pairs` contains that exact slot
(`kernel_io.rs:438-449`). Such a claim goes through the existing
`end_association` then `release_slot` order, which drains the queue,
disconnects the socket, clears readiness and returns the map slot
(`kernel_io.rs:1099-1113`).

An exact-cookie claim that already crossed the owner boundary remains in the
existing owner event path. `SocketReleased` is emitted for published pending
or paired claims, and `GuestFlowOwner` releases those through its existing
association close/abort/end-record path (`owner.rs:282-293, 829-892`). This
preserves the distinction between an adapter-owned claim and an owner-owned
association. It also avoids treating a retained historical vsock handle as a
current association.

The implementation has no new failure taxonomy. The scan's private `Option`
fallback preserves the pre-existing event path for an unavailable adapter map;
the initialized production transport already requires the existing BPF maps.
No string diagnostic substitutes for a designed typed cause, and no public
error variant or payload was introduced.

The source-local Sim changes model the same already-approved resident claim
boundary. `prepublication_slots_for_cookie` requires held readiness and no
current flow, vsock or pair (`sim.rs:734-750`); reclamation uses the existing
`GuestOwnerIo::end_association` and `release_slot` effects
(`sim.rs:769-778`). Published pending and paired claims remain on the normal
owner path. The Quint `kernelSlots` variable and `KernelSlot` type were
already part of the accepted model; the driver now compares that existing
resident projection separately from the QPROJ-5 owner projection
(`quint_udp_slots.rs:619-768`). The named actions, invariants, runs and
identities remain unchanged.

The strengthened source-local burst scenario asserts cleanup while readiness
is still held, before the owner can invoke its missing-cookie fallback
(`crates/overdrive-init/src/guest_flow/tests/udp_slots.rs:242-317`). It checks
the full discard/disconnect/return order, absence of a flow or pair, no
request side effect, and no old datagram at the recycled owner's pair.

### RESET-EXIT, NO-SURVIVE and no fault hook

The fault matrix uses the existing production `Lane::boot`, public deploy
handler and real workload composition. The Service helper in
`vsock_support.rs:324-343` is private test support and declares only the
admission listener required by the Service shape. It does not add a production
restart mechanism or alter the Job contract. The five CLI bodies retain their
existing assertions; the only behavior-level correction is the accepted
Service fixture for the guest-reset successor.

The test runner's exact selector maps only the six existing identities in
`fwd_guest_suite.rs:1184-1230,2177-2299`. It stages the existing Cargo test
ELFs through `xtask/src/guest_kit_prepare.rs:154-160` and invokes each exact
body inside a fresh disposable outer GuestKit guest. No `run_server` fault
field, production VM-driver hook, link detach injection or alternative VMM
command line was added. D19-LINKS remains represented by the explicit absence
of a detach stimulus.

### Leg-F repair and TPROXY boundary

The worker test creates a route-disjoint workload namespace and veth, assigns
test-owned CIDR leases, appends exactly the production egress TPROXY rule
expression, and assigns a destination with no listener
(`crates/overdrive-worker/tests/leg_f_repair.rs:254-557`). Both the EMFILE
stimulus and the post-repair connection traverse this capture path. The test
checks rule identity/counter and handle ownership before removal, removes the
fixture rule by handle before repair/audit, and preserves foreign state.

`reserve_dense_fd_table` creates descriptors before lowering the process soft
limit (`leg_f_repair.rs:209-233`). The final receipt observed
`TaskFailed { leg: F, source: ... code: 24 }` with the listener still LISTEN;
after restoring the limit, repair returned `Ok(())`, the inode changed and
the old descriptor disappeared. The second captured connection was accepted
and closed within the bound, then the worker audit passed. This is the
required existing worker-level Tier-3 cell, not a new production mechanism.

## Test integrity, external validity and Contract Shape

The six exact step identities all retain `/// CONTRACT_SHAPE: bounded-change.`
and their domain-language names. The source diff shows no assertion deletion,
assertion weakening, test skip, runtime-skip bailout, fixture end-state
seeding or mock-dominated SUT. Marker removal activates the DISTILL-authored
bodies; the worker's support correction is required to drive the already
accepted TPROXY path and retains the EMFILE, LISTEN, inode, descriptor, repair
and audit assertions.

The native acceptance set has six distinct observable behaviors and six active
bodies, below the software-crafter budget of 12 (`2 × 6`). The 26 owner/Sim
UDP cells are existing DISTILL support identities reviewed separately; this
step does not add test identities. The exact GuestKit runner executes one
selected body per invocation and fails closed on zero selection, wrong body
count, nonzero exit or recognized runtime skip. Tests enter through the real
production `serve`/`deploy` composition for CLI cells and through the real
`HostMtlsIntercept` worker composition for leg F. The source-local tests use
the established owner driving port and Sim driven-port boundary.

Quality-gate disposition:

| Gate | Result | Evidence |
| --- | --- | --- |
| G1 single acceptance boundary | PASS | Exact GuestKit selector runs one existing body per invocation; the six required step identities are the fixed source manifest. |
| G2 valid RED | PASS | Native UDP/worker RED receipts and corrected guest-reset RED reached semantic assertions; DES records three RED failures. |
| G3 assertion failure | PASS | The UDP body failed its `Terminated` oracle after real `EPERM`; worker and reset failures reached their stated semantic assertions. |
| G4 no domain mocks | PASS | Production compositions and real kernel/GuestKit adapters are used; Sim doubles are only the approved driven-port support boundary. |
| G5 business/domain language | PASS | Owner death, RESET-EXIT, NO-SURVIVE, slot release, EMFILE, TPROXY capture, far-reader backpressure and repair are named precisely. |
| G6 all green after implementation | PASS | Five CLI GuestKit receipts and one worker GuestKit receipt each report one exact body passed. |
| G7 100% focused pass | PASS | Final receipts are exit 0, `1 passed; 0 failed; 0 ignored`, with zero canonical-117 credit and zero runtime skips. |
| G8 test budget | PASS | Six native behaviors, six bodies, budget 12. Existing 26-cell support inventory is unchanged in identity. |
| G9 no test modification to accommodate implementation | PASS | Only pending markers, the approved Service fixture shape, and approved TPROXY support changed; required assertions remain. |

The Contract Shape checks therefore pass mechanically and semantically. No
test name uses the banned implementation-oriented forms, no new test lacks a
declaration, and the acceptance assertions are observable outcomes.

## Verification evidence

### Safe native GuestKit receipts

The final five CLI receipts and one worker receipt all run through the exact
GuestKit route. Each receipt records a disposable outer Cloud Hypervisor guest,
guest kernel `7.0.0-38-generic`, nested device evidence where required,
`canonical_117_credit=0`, no runtime skips and cleanup completion. The focused
CLI run `3fe1de0c88e55b5795bb1953` reports the exact UDP body as `1 passed; 0
failed; 0 ignored`, source manifest five, and outer VMM reaped. The other CLI
receipts are:

| Identity | Receipt | Result |
| --- | --- | --- |
| Owner death / NO-SURVIVE | `bdc5ea0b1cda6b9e5c2717a7` | one exact body passed |
| VMM SIGKILL | `d109b9e974f4571bf7b29a12` | one exact body passed |
| Guest reset / RESET-EXIT | `37650e5166b2a1ae2992a003` | one exact body passed |
| Far-reader stall | `1f6bd6e5d87b3e346134ab57` | one exact body passed |
| UDP churn | `3fe1de0c88e55b5795bb1953` | one exact body passed |
| Leg-F repair | `b74d312a08ef94d10c47b2c2` | one exact worker body passed |

The raw output for the invalid historical direct Metal executions is retained
under `.context/netns-density-295/14-02/raw/` and classified in
`incident-evidence.md`. Those runs receive zero acceptance credit. The final
receipts are independent GuestKit executions and do not rely on host-side
netns, veth, nft, cgroup, route, sysctl, BPF or module state.

### Formal, Sim and source checks

The owner/Sim support review retains 26 bounded UDP cells passing at
`98d5584ab3599d8db3f07622`. Because `udp_slots.qnt` changed as an atomic
formal input, the required full three-job record was rerun. Receipt
`fc8443bcede4d27dd0ff0a60` records 279/279 checks matching `expect`, including
all 21 UDP checks; `9ea4762a840e20a5b4851754` verifies the generated evidence.
The formal check `udp-slots-hazard-late-paired-by-slot-route` took 191.905
seconds while still marked `ci=true`; this is an inherited timing/evidence
limit. No CI-budget compliance credit is claimed, and no configuration or
timeout waiver is inferred.

The final scoped source gates are:

| Check | Result |
| --- | --- |
| `cargo fmt --all -- --check` | PASS, receipt `c0a1c002534ec4bbac4c171a` |
| `cargo check --workspace --all-targets --features integration-tests` | PASS, receipt `ea3a4bd8ffc61591dbead200` |
| `cargo check -p overdrive-init --features integration-tests` | PASS, receipt `c005bb82e71a05391561d36e` |
| `cargo clippy -p overdrive-init --features integration-tests --all-targets -- -D warnings` | PASS, receipt `ea9a18a27aa90f17f9e6daaf` |
| focused CLI/worker Clippy | PASS, final receipts `final-cli-clippy`, `final-worker-clippy` |

The earlier workspace check failed only because the staged BPF artifact was
permission denied; the final workspace check passed. The unqualified
workspace Clippy baseline remains a pre-existing warning ledger in
`overdrive-control-plane` (`expect_used` and `unchecked_time_subtraction`);
the step-specific Clippy receipts are the relevant clean gates. This review
does not relabel the baseline as a step pass.

## Findings and dispositions

### `praise:` exact lifecycle and boundary preservation

The implementation keeps the resident kernel claim, published owner state,
pending association and paired association distinct. It reuses the established
cleanup order and existing owner event path, while the final exact receipts
prove all six identities inside the safe disposable GuestKit boundary. The
worker support correction has unusually strong causal checks: the destination
has no listener, the exact production rule is inspected by normalized identity,
EMFILE is observed as errno 24, and the post-repair connection must be
accepted and closed.

### Evidence limitation E-01 — no blocking finding: lost-report prepublication audit parity

The current production `live_cookies` path receives cookies from the published
owner table and does not itself scan a kernel-only `SLOT_META` claim
(`owner.rs:908-925`, `kernel_io.rs:1128-1134`). The Sim audit deliberately
models the resident prepublication claim, and the formal record checks it.
The retained support review correctly classifies production parity for the
specific schedule “release report lost while readiness is still held” as
coverage-only and unproven. Under the repository reachability gate there is no
bounded real production reproduction of that ordering, so this static
discrepancy is not promoted to a defect and no production design change is
requested. D25/K-B2 remains a mandatory contract; this evidence limit is not
a waiver or a claim of production proof.

### Evidence limitation E-02 — no blocking finding: formal CI timing flag

The full 279-check record passes, but the inherited
`udp-slots-hazard-late-paired-by-slot-route` check took 191.905 seconds while
marked `ci=true`. The support review records this honestly and grants no
under-60-second CI eligibility credit. No remediation is required for this
bounded step review because the formal result itself is complete and no
timeout/configuration waiver was used.

### Historical evidence disposition

The eight unsafe direct Metal receipts, stale-process cleanup and residual
`ip overdrive-mtls` observation remain classified in
`incident-evidence.md`. They are not relabeled as GuestKit evidence and do not
contribute acceptance credit. The six later safe receipts independently prove
the current bodies. No host cleanup or mutation testing was performed by this
review.

There are no blocking findings. No remediation iteration is required.

## Iteration 1 verdict

**APPROVED.** The cumulative implementation satisfies the exact 14-02
acceptance criteria and the approved D25, RESET-EXIT, NO-SURVIVE and
D19-LINKS contracts within the evidence boundaries authorized for this step.
The UDP fix is necessary and behaviorally supported by semantic RED followed
by the final safe GuestKit pass; it preserves published pending/paired owner
paths and adds no public surface. The leg-F repair proves the required
TPROXY-captured EMFILE path and release-before-rebind lifecycle. DES, commit
trailers, Contract Shape declarations, test integrity, formal evidence and
safe native boundaries all pass. The two explicit evidence limits remain
recorded without weakening any accepted MUST or granting unsupported credit.
