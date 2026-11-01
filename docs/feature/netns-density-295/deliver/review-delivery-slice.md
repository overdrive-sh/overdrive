# DELIVER Whole-Slice Review — `netns-density-295`

## Review metadata

| Field | Value |
|---|---|
| Review type | Fresh independent final DELIVER slice review |
| Reviewed branch / HEAD | `marcus-sa/flow-quota-240s-cap` / `00ce922c43b9743651cfda535af006f2393abbe3` |
| Review scope | New executable phases 11–14, all 18 steps, including remediation and cross-step integration |
| Excluded scope | Frozen phases 01–04 and superseded phases 05–10; their historical gaps are preserved and are not reclassified here |
| Authority | Approved `deliver/roadmap.json`, `feature-delta.md`, accepted ADR-0187–ADR-0211 and approved amendments, Quint sources/evidence, DISTILL scenarios, source and native receipts, and final step reviews |
| Worktree boundary | Pre-existing dirty `AGENTS.md` preserved; no source, test, DES, roadmap, host or workflow record was changed by this review before this artifact |

This review derives implementation boundaries from the actual step history and
reviewed commits. It does not use `origin/main` as a boundary and does not
grant acceptance credit to an old failed receipt, a host-boundary incident, a
benchmark, or a nested/virtualized host run.

## Whole-slice conclusion

The delivered production slice is coherent with the accepted contract. The
attachment cut is implemented through the production composition root, the
guest-neutral flow protocol and the in-kernel forwarder. The accepted public
API, wire and persisted shapes, ownership, lifecycle, async completion and
typed semantic error mappings are consistent across phases 11–14. No source
defect, API divergence, test-theater defect, test weakening or unproven
control-plane remedy was found in the final implementation after accounting
for the approved queued-end regression.

The full workflow closure is **NEEDS_REVISION for mechanical artifact
completeness only**. Four expected per-step review artifacts are absent and a
roadmap validation paragraph still describes the earlier STEP 14-01 RED state
after its approved GREEN implementation. These gaps do not imply a production
failure and are recorded below without inventing historical reviews or
changing the roadmap.

## Accepted cross-step contract and implementation assessment

### Composition and ownership

The production path is internally consistent:

1. `run_server` performs cgroup preflight, the guest-flow startup probe and
   VMM capability probing before boot effects. It composes one `OwnerDial`, one
   `GuestNetworkOwner`, the VMM beacon intake and the configured node vsock
   ports.
2. `boot_server` composes the mTLS enforcement over that same dial, builds the
   shared owner, reclaims old VMs before the stale owner sweep, writes the
   shared mTLS table before forwarding opens, converges the flow listeners and
   gateway route, then reads the owner-bound flow ports back into state.
3. The action shim assigns one `GuestTransportAssignment` from the
   `GuestAddressPool`, provisions the CID/token, writes the assignment into
   `AllocationSpec.transport`, starts the VMM, publishes `Running`, installs
   allocation mTLS state, activates under the EXEC claim, and releases EXEC
   only after the promised effect is complete. The relevant ordering is visible
   at `crates/overdrive-control-plane/src/action_shim/mod.rs:1460-1474` and
   `:1650-1685`.
4. Stop/reclaim retires the lease, stops the VMM, stops mTLS ownership,
   awaits guest-network teardown and releases the pool lease only after the
   owner reports an empty complement. `teardown_guest_network` preserves this
   order at `action_shim/mod.rs:1587-1602`; reclaim uses the same protocol at
   `:1605-1647`.

The owner retains the forwarder context by value; context release ends its
pairs and watches. The guest owns its sockets, capture programs, forwarder
context and guest-side kernel effects until guest exit. The VMM alone opens
`/dev/vhost-vsock`, and `CloudHypervisorVmm` renders exactly
`cid=<leased>,backend=vhost-kernel` at `overdrive-host/src/vmm.rs:368-375` and
`:532-539`. No process-local bridge, TAP, per-VM netns or VM override seam is
used by the VM attachment path. Reusable bridge/TAP libraries and unrelated
tests remain where the accepted scope permits them.

### Exact interfaces and typed failures

The final source matches the approved interface contracts:

- `AllocationSpec.transport` carries `GuestTransportAssignment`; the VM
  driver consumes its CID, launch token and flow ports.
- `NodeVsockPortConfig` validates fixed or kernel-assigned node listeners and
  `GuestFlowPorts` reports the actual stream/control ports. The guest receives
  beacon, stream and control values in `overdrive.transport=vsock2`.
- `GuestNetworkError` has the accepted exact post-cut variants, including the
  nested `ForwarderError`, `PostconditionMismatch`, prefix/VIP refusals, fd
  budget and owner placement failures. No missing semantic cause is carried
  only in a string.
- `SharedGuestNetworkFailStopCause` contains exactly the approved recovery,
  quiescence and supervisor terminal causes. No bridge/TAP kill-failure cause
  was retained or invented.
- Guest setup maps each designed failure to its approved private
  `GuestTransportSetupError` representation; the host projects setup failure
  through the existing pre-READY VMM-exit path.
- Forwarder inventory retains `queued_ends` as an internal observable without
  adding it to the pinned `GuestNetworkFact` public shape. The approved seeded
  Sim regression proves the private arithmetic is necessary without changing
  the interface contract.

The public `run_server` and supplied-owner composition signatures, VMM port,
driver constructor, guest owner ports, wire codecs, listener refusal forms,
probe run sequence and persisted row shapes were checked against the feature
delta and accepted ADRs. No new public method, type, enum variant, trait,
parameter, wire field or persisted field was introduced to make a test pass.

### Cross-boundary test integrity

The accepted evidence keeps the required layers separate. Rust tests use the
production composition in-process; GuestKit expectations drive the built
product and observe external surfaces; the source-local Sim regression enters
the production owner/supervisor boundary and uses only the existing driven
Sim port. No expectation runner invokes `cargo test`, imports an
`overdrive-*` crate or duplicates the implementation in another language.

The final selected tests retain their Contract Shape declarations and business
outcome anchors. The final 64-VM bodies are existing identities with bounded
live-row address observation; no terminal address was reconstructed after a
failed allocation. The queued-end body is the one user-approved additional
seeded regression and proves parent failure/restored pass with identical seed,
action trace, source identity and cleanup.

## Evidence reviewed

### Step and commit boundaries

The implementation chain was reviewed at the following actual boundaries:

| Step | Implementation and remediation boundary | Final on-disk review |
|---|---|---|
| 11-01 | `cb81320cb6e0` through warning-gate remediation `5a114fbd4bfd` | `review-11-01.md`, APPROVED |
| 11-02 | `c60ccedd946e` | **Missing** — see F-01 |
| 11-03 | `c3ed14f8e98f` | **Missing** — see F-01 |
| 11-04 | `4aeb1bf447cc` plus local provisioning/lint corrections | **Missing** — see F-01 |
| 11-05 | `b8226c05ea0b` | **Missing** — see F-01 |
| 11-06 | `2f92c925d374`, remediation `13b6a2d4d5b5` and `6e8d2df862fd` | `review-11-06.md`, APPROVED after three iterations |
| 12-01 | `7105b5807b1d`, API correction `83419d30825f` | `review-12-01.md` and replacement-proof review, APPROVED |
| 13-01 | `9c23d8c63b22` | `review-13-01.md`, APPROVED |
| 13-02 | `60fd13746a9b` | `review-13-02.md`, APPROVED |
| 13-03 | `8ff253c0f6b5` plus native proof | `review-13-03.md`, APPROVED after two iterations |
| 13-04 | `ec3bdc484f17` plus fixture/verification remediation | `review-13-04.md`, APPROVED after two iterations |
| 13-05 | `a4c07cdf047c` plus route-label remediation | `review-13-05.md`, APPROVED after two iterations |
| 13-06 | `b51d5f3dc1e3` plus GuestKit/normal-gate remediation | `review-13-06.md`, APPROVED after two iterations |
| 14-01 | `fe22d261dfd9` through `bc832a3e0e6b` | `review-14-01.md`, APPROVED after two iterations |
| 14-02 | `abfd83d38dbd` | `review-14-02.md`, APPROVED |
| 14-03 | `5aaf29c96078` | `review-14-03.md`, APPROVED |
| 14-04 | `8e017a3e4bd9` through DES repair `9b0bb15387de` | `review-14-04.md`, APPROVED after three iterations |
| 14-05 | `ba85e05c321f`, queued-end correction/regression `9dc7bb9ba326`, `68b0f7b10c4b`, `846d8ec02659` | `review-14-05.md`, APPROVED after two iterations |

All accepted commits preserve the Marcus author and the exact
`Co-Authored-By: Codex <codex@openai.com>` trailer. The final branch remains
`marcus-sa/flow-quota-240s-cap`. The final worktree audit before this artifact
contained only the pre-existing dirty `AGENTS.md`.

### DES and marker evidence

`execution-log.json` contains 296 append-only events. Each new step has a
recorded RED/GREEN/COMMIT history; intermediate blocked attempts and the
approved 14-05 parent-fail/restored-pass regression are preserved. The latest
accepted implementation phases have canonical `PASS` values where the phase
represents the current source. The 14-05 `RED` `FAIL` event at
`2026-11-01T20:40:25Z` is the deliberately failing parent-source regression
receipt, followed by restored-source GREEN and COMMIT PASS; it is not a claim
that the accepted source is red.

The active feature marker inventory has no `#[ignore = "pending DELIVER ..."]`
or attachment-cut pending marker. The only enduring feature-linked ignore is
the approved `cid_lease` `#325` window. Unrelated legacy ignores and the
unrelated `xtask` test marker remain untouched. The frozen 01–04 DES gaps are
not rewritten or promoted to new-scope findings.

### Formal and deterministic evidence

The current Quint input hashes match the checked-in files without a rerun:

| Evidence | Result and boundary |
|---|---|
| `specs/quint/guest-flow-owner` | Run `2228824-1793511799.752`; 279/279 checks match expected outcomes (209 holds, 70 expected violations); all 28 input SHA-256 values match current files; `verify-evidence` PASS retained |
| `specs/quint/probe-count` | 31/31 checks match expected outcomes; input hashes match current files; prior approved evidence retained |
| `specs/quint/tcp-capture-pending` | 8/8 checks match expected outcomes; input hashes match current files; prior approved evidence retained |
| Deterministic Sim | Approved seeded pool, owner, flow-quota, fwd-pair and quiescence boundaries retained; the new queued-end parent fail is `2c362955672f8c2357a11a60`, restored source pass is `fbbd698701ac86754a28e70d`, seed `11400714819323198485`, with cleanup awaited before assertions |

The pre-falsification formal record in the feature delta is historical context;
the current source-hash checks and accepted review evidence are the freshness
boundary. No unchanged formal input was rerun.

### Native and black-box evidence

The native evidence is explicitly bounded to disposable production
Cloud-Hypervisor/GuestKit guests on the approved Linux 7.0 line. Physical Metal
transport is not treated as isolation. The final evidence includes:

- forwarder module build/load, unload and guest-boundary evidence from 11-01,
  11-05 and 11-06;
- the V-6 walking skeleton's exact retained identity set, 117/117 pass,
  zero runtime skips and clean GuestKit cleanup;
- 14-02's safe fault-matrix receipts, with historical unsafe direct-host
  receipts retained as excluded evidence;
- 14-03's in-process required-port fail-stop/recovery conformance;
- 14-04's canonical cgroup-default run `9c8fbc97ef29018ca5426e5c`, exactly
  119/119, zero failures/required ignores/runtime skips, and the qualified
  `mtls-relay` run `1e8aba12ce7f53f7be0560ff`, exactly 46/46;
- 14-05's focused correctness runs `48a2a4804cdb008122887ddc` and
  `bf76f2e6bf9a6f83dd3fe1c0`, both exit 0 with one selected body, zero
  ignored, 64 and 122 cleanup completions, zero canonical-manifest credit,
  and source/staged ELF SHA-256 `abe4ef403f6ae03549a9aa01e645b7b5bfe787d414acc4d7573b1a069a72a547`.

No density/performance claim is made from 14-05. The final 14-04 source and
native receipts are the current non-regression evidence; the retained
advisory 191.905-second CI hazard check is not represented as a waived gate.

## Findings and dispositions

### F-01 — Blocker for mechanical finalization: four executable-step review artifacts are absent

The required paths
`deliver/review-11-02.md`, `review-11-03.md`, `review-11-04.md` and
`review-11-05.md` do not exist. `find`, `git ls-tree -r HEAD`, `git log --all
-- <path>` and the feature context contain no alternate full review artifact
under another name. The context dispatches record that the delivery plan
requested one final full-slice review and did not persist routine reviews for
these four steps, but that is not a substitute for the repository's current
on-disk DELIVER review convention.

The absence does not prove a source defect. The code, DES history, step
receipts, accepted design and later cross-step consumers were independently
read here, and the four steps are included in the whole-slice assessment.
However, an approval or timestamp for any missing historical review would be
fabricated. Finalization must either restore authentic reviewer artifacts from
their owner or record an explicitly authorized mechanical disposition that
this whole-slice review is the bounded replacement. No source, test or
roadmap change is prescribed by this finding.

**Disposition:** OPEN; blocks a clean mechanical finalization record, not the
production correctness conclusion.

### F-02 — High for evidence closure: roadmap validation prose is stale after STEP 14-01 GREEN

`deliver/roadmap.json:1702-1705` has `validation.status = "approved"` but its
reviewer text still says the exact runner is an honest semantic RED and that
there is no 117-pass receipt or completed DELIVER review. The current
`review-14-01.md`, DES log, implementation/remediation commits and retained
117-pass native receipt establish the opposite for the implemented step. The
status and accepted UDP deferral are still valid; the stale sentence is an
evidence-provenance contradiction.

This is documentation state, not a design or production defect. The artifact
must be corrected or explicitly dispositioned by the owner before finalization
can claim that the roadmap validation describes the current delivered state.
This reviewer does not edit `roadmap.json`.

**Disposition:** OPEN; blocks an unqualified closure statement, not source
acceptance.

## Material limitations and accepted deferrals

| Item | Current disposition |
|---|---|
| Service-VIP UDP delivery to a VM backend | Explicitly deferred to GH #310 by the user; no substitute host backend or silent acceptance credit |
| 64-VM density measurements, throughput, latency, memory and CAP-295-A receipts | Explicitly deferred to GH #326; 14-05 proves correctness only |
| Generation replacement lost at `serve` crash | GH #325; approved `cid_lease` model window remains ignored and is not fixed here |
| vDPA/SR-IOV feasibility | GH #329; no hardware enablement or architecture change |
| Netlink functionality moved into core | GH #332; current adapter ownership remains |
| Domain naming migration | GH #333; `.claude/rules/domain-naming.md` is the accepted transition policy; existing required filenames/history remain |
| TOML loading for node vsock ports | GH #327; ports are configured through the accepted `ServerConfig` shape in this slice |
| Non-Linux guests | GH #321; host protocol remains guest-neutral, Linux guest adaptation is the delivered boundary |
| Existing baselines | E6 `guest_stack_mtls_egress::fault_fixture`, known quiescence seeds and other named baselines remain exact historical evidence; no blanket green claim |
| Unsafe or host-boundary receipts | Retained as incidents/diagnostics and excluded from acceptance credit; current credit comes from disposable GuestKit/native receipts or safe userspace checks |
| Mutation, KASAN and performance benchmark gates | Not run by explicit user instruction; no mutation score, KASAN claim or benchmark result is claimed |

## Closure gates remaining

The following are intentionally left as closure conditions rather than hidden
inside this approval:

- Resolve or explicitly authorize the bounded replacement for the four absent
  per-step review artifacts (F-01).
- Correct or disposition the stale roadmap validation paragraph (F-02).
- Keep frozen 04-02/04-03/04-04 DES gaps and any unrelated ignores preserved;
  do not manufacture historical events.
- Keep mutation testing, KASAN and GH #326 measurements outside this feature's
  evidence unless the user changes the explicit deferral/prohibition.

No new native, formal, mutation or host run is necessary for this review:
current input hashes, source/staged ELF identity, final receipts and approved
reproductions are sufficient at their stated boundaries.

## Final verdict

**NEEDS_REVISION — mechanical closure only.** The integrated implementation
and its evidence are accepted within the explicitly listed boundaries; no
production or test remediation is requested. The feature must not be reported
as mechanically finalized until F-01 and F-02 receive an honest owner-level
disposition. The frozen history, accepted deferrals and all historical failure
evidence remain intact.

## Review iteration 2 — mechanical closure remediation

### Scope and authority

This iteration reviews only the closure changes made after iteration 1:

- `review-11-02.md`, `review-11-03.md`, `review-11-04.md` and
  `review-11-05.md` are now present in the feature's `deliver/` directory.
- The four artifacts are current-date closure reviews. They explicitly state
  that they do not claim a reviewer decision at the historical implementation
  or completion time. Their final verdicts are APPROVED; 11-03 and 11-04
  preserve their first iteration findings and approved second-iteration
  remediation dispositions.
- The only executable-source changes associated with those findings are the
  nine exact Contract Shape rustdoc declarations already persisted by the
  bounded metadata commits: five pure-function declarations in
  `vm_beacon_roundtrip.rs`, three bounded-change declarations in
  `vmm_equivalence.rs`, and one bounded-change declaration in
  `vm_host_state_equivalence.rs`. The bodies, assertions, selectors,
  production code and public API are unchanged. The commits are
  `6a255656ef59` (Step-Id 11-03) and `36ff03d51e91` (Step-Id 11-04), each
  with Marcus's author and the exact Codex trailer.
- `roadmap.json` changed only the stale validation reviewer paragraph. The
  approved status, approval time, prior approval chain, pending scope, design
  contract and explicit #310 UDP deferral are unchanged. The new paragraph
  points to the approved 14-01, 14-04 and 14-05 evidence.

No native, formal, mutation, KASAN, Lima or host run was needed. The
metadata-only test corrections were reviewed with formatting and diff checks;
the current step evidence and input hashes remain the evidence assessed in
iteration 1. The current worktree also contains the pre-existing dirty
`AGENTS.md`, the owner roadmap edit, the four review artifacts and this
review artifact; no unrelated work was adopted.

### F-01 disposition — CLOSED

The four previously absent paths now exist and were inspected in full:

| Artifact | Current review result |
|---|---|
| `review-11-02.md` | APPROVED; current closure review, no historical execution claim |
| `review-11-03.md` | Iteration 1 NEEDS_REVISION for five missing declarations; iteration 2 adds only those declarations and is APPROVED |
| `review-11-04.md` | Iteration 1 NEEDS_REVISION for four missing/stale declarations; iteration 2 adds only those declarations and is APPROVED |
| `review-11-05.md` | APPROVED; current closure review, no historical execution claim |

The current artifacts provide scope, evidence, findings, remediation
dispositions and final verdicts. The 11-03 and 11-04 metadata findings are
now closed by exact declarations; no executable behavior changed. F-01 is
**CLOSED**.

### F-02 disposition — CLOSED

The exact `roadmap.json` diff replaces the earlier sentence claiming that
STEP 14-01 remains RED and has no 117-pass receipt with the current approved
14-01 review and the current 117/119/46/64 evidence references. No normative
contract, accepted status, approval timestamp, pending scope, design decision
or UDP deferral changed. F-02 is **CLOSED**.

### New closure review finding — preserved frozen-history integrity gate

The installed `des-verify-integrity` run still exits 1 with exactly the frozen
history findings supplied by the orchestrator: no entries for 04-03 and 04-04,
and 04-02 incomplete because its COMMIT phase is missing. These are phases
01–04 frozen history, outside the delivered 11–14 slice. The user explicitly
forbids fabricating phases, timestamps, waivers or history rewrites, so this
iteration does not change them.

This is a genuine repository-wide archive/integrity limitation. It is not a
finding against any of the 18 delivered 11–14 steps, and it does not reopen
their approved reviews. The current slice can be approved while the overall
feature history remains unqualified for a global zero-violation finalization.

**Disposition:** OPEN as a preserved global historical gate; out of scope for
the requested 11–14 slice and not remediated here.

## Iteration 2 verdict

**APPROVED — delivered phases 11–14 only.** The two mechanical closure
findings from iteration 1 are closed by authentic current-date review
artifacts, exact metadata-only Contract Shape declarations, and a one-field
roadmap evidence update. The integrated implementation, public API, typed
errors, lifecycle ownership, deterministic/formal evidence, disposable native
boundaries, accepted deferrals and historical failure qualifications remain
honest and unchanged.

This approval is bounded to the completed 18-step 11–14 slice. It does not
mark the feature globally finalized and does not waive or repair the frozen
04-02/04-03/04-04 DES integrity gaps. The global archive/integrity limitation
must remain visible to finalization and no historical event may be invented.
