# Combined DISCUSS + DESIGN Review - transparent-mtls-host-socket

**Reviewer**: Codex, applying DISCUSS handoff / DoR checks plus architecture review criteria \
**Date**: 2026-06-12 \
**Scope**: `feature-delta.md`, DISCUSS wave decisions, product/local journeys,
J-SEC-003, Sam persona, slices 00-05, ADR-0069, C4 diagrams, design wave
decisions, upstream-change notes, spike findings, product outcome registry, and
existing review artifacts. \
**Verdict**: **rejected_pending_revisions**

Review stance: the kernel feasibility is not being relitigated. The user premise
is accepted: the spikes have verified the mechanism. This pass checks whether the
DISCUSS and DESIGN artifacts now teach one consistent, ready-to-DISTILL/DELIVER
contract.

Most prior blockers are resolved. The artifacts are re-grounded on ADR-0069's
bidirectional universal agent-light L4 proxy; Sam's persona and J-SEC-003 now
teach the proxy model; the C4 outbound peer endpoint is now the peer-side agent,
not the workload; OQ-1 is accepted in the feature-local DESIGN artifacts; OQ-2 is
resolved; the inbound half, authn-only v1 boundary, resource limits, and pump
supervision are represented.

The handoff is still not clear because one product architecture SSOT still carries
stale approval/blocker state, and a few proof-status/back-propagation statements
can still send an orchestrator or crafter toward the wrong state model.

## Findings

### F1 - High - Product architecture brief still contradicts the accepted OQ-1 and #230 state

The feature-local DESIGN artifacts say the `MtlsEnforcement` contract is accepted,
but the product architecture brief still contains stale "open/pending" language.

Evidence:

- `docs/feature/transparent-mtls-host-socket/feature-delta.md:1504` says OQ-1 is
  accepted, and `:1591`-`:1623` marks the port contract `ACCEPTED`.
- `docs/feature/transparent-mtls-host-socket/design/wave-decisions.md:80`-`:88`
  says the exact contract is accepted and no longer a blocker.
- `docs/product/architecture/brief.md:5613`-`:5619` still says the method shapes
  are pinned in feature-delta and that an "open item OQ-1 below" exists.
- `docs/product/architecture/brief.md:5690` still says the contract remains
  "PROPOSAL - PENDING USER APPROVAL" and that operator-tunable limits are a blocker
  with no issue created. Current feature-delta says #230 exists and v1 limits are
  compile-time defaults (`feature-delta.md:2829`-`:2833`).

Impact: the architecture brief is a product SSOT. A DELIVER orchestrator reading it
can incorrectly block on OQ-1 again or try to create an already-created #230-style
deferral.

**Required revision**: update the active brief section and changelog row to say
OQ-1 is accepted, the exact feature-delta contract is binding, #230 tracks
operator tunability, and no contract-approval blocker remains.

### F2 - Medium - Some proof-status language is broader than the Slice 00 integration contract

The best current phrasing is: the mechanism is spike-proven; the inbound composed
flow is spike-proven; outbound one-flow composition, bidirectional response legs,
and real netns/veth + cgroup isolation remain Slice 00 integration gaps. Several
artifacts say this correctly, but two architecture surfaces still use broader
"both directions proven" wording.

Evidence:

- ADR-0069 states the narrow distinction clearly: three composition gaps remain at
  `docs/product/architecture/adr-0069-transparent-mtls-universal-agent-light-l4-proxy.md:80`-`:100`.
- Slice 00 states the same integration gate at
  `docs/feature/transparent-mtls-host-socket/slices/slice-00-composed-proxy-walking-skeleton.md:73`-`:110`.
- `docs/product/architecture/brief.md:5522`-`:5530` says both directions are
  spike-proven without naming the remaining response/netns gaps.
- `docs/feature/transparent-mtls-host-socket/design/c4-diagrams.md:8`-`:10` says
  both directions are real-kernel proven; that is true for primitives/inbound, but
  too easy to read as full bidirectional composition.

Impact: DISTILL could under-specify Slice 00 by treating the remaining gaps as
already closed.

**Required revision**: replace broad "both directions proven" phrases with the
precise distinction: "inbound composed path and per-direction primitives are
proven; Slice 00 composes outbound one-flow, bidirectional response legs, and
real netns/veth/cgroup isolation."

### F3 - Medium - Feature-delta handoff still says back-propagation needs to happen

The back-propagation appears complete in the current working tree, but the
feature-delta DESIGN handoff still reads as if the product owner must do it later.

Evidence:

- `docs/feature/transparent-mtls-host-socket/feature-delta.md:1586`-`:1589` says
  J-SEC-003 and slices 00-05 "need re-grounding" and that this is only flagged for
  the product owner.
- `docs/feature/transparent-mtls-host-socket/design/upstream-changes.md:3`-`:19`
  says the back-propagation is complete and every listed product/slice/journey file
  has been actioned.
- Current J-SEC-003 and Sam persona are already re-grounded
  (`docs/product/jobs.yaml:352`-`:416`,
  `docs/product/personas/sam-platform-security-engineer.yaml:100`-`:130`).

Impact: this is not a design flaw, but it is handoff ambiguity. A later agent can
repeat or delay already-completed work.

**Required revision**: update the feature-delta handoff to say back-propagation is
complete and point to `design/upstream-changes.md` as the completed record.

### F4 - Low - Outcome registry classifies a future DELIVER gate as `kind: spike`

`OUT-MTLS-COMPOSED-PROXY-SKELETON` now describes the first blocking DELIVER slice,
not a completed spike.

Evidence:

- `docs/product/outcomes/registry.yaml:232`-`:250` has
  `kind: spike` while the summary calls it "the FIRST, BLOCKING DELIVER slice" and
  an integration gate.
- `docs/product/outcomes/registry.yaml:260`-`:268` describes a PASS condition for
  the future bidirectional netns/veth gate.

Impact: lower risk than F1-F3, but the registry status is easy to misread as
already-spiked completion rather than a blocking acceptance invariant.

**Recommended revision**: reclassify this entry as an invariant/specification, or
add explicit status wording that it is the Slice 00 acceptance gate, not a completed
spike result.

## Passed Checks

- YAML parsing passed for `docs/product/jobs.yaml`,
  `docs/product/personas/sam-platform-security-engineer.yaml`,
  `docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml`,
  `docs/feature/transparent-mtls-host-socket/discuss/journey-enforce-transparent-mtls.yaml`,
  and `docs/product/outcomes/registry.yaml`.
- Sam's product persona, J-SEC-003, product journey, feature-local journey, and
  slices 00-05 are re-grounded to the ADR-0069 agent-light proxy model.
- The C4 outbound L3 peer endpoint now correctly names the peer-side agent as the
  mTLS endpoint and keeps the peer workload identity-unaware.
- The `InterceptedConnection` wording now distinguishes inbound leg C from a
  plaintext fd.
- OQ-1 is accepted in the feature-local DESIGN artifacts; OQ-2 is resolved.
- Inbound path, guest-stack staging, resource limits, pump supervision, and
  chain-to-bundle-only authn are represented in the DESIGN artifacts.
- Historical review artifacts carry superseded banners, so their stale rejected
  verdicts are less likely to be mistaken for current state.

## Approval Status

`rejected_pending_revisions`

Minimum bar to proceed:

1. Fix `docs/product/architecture/brief.md` so OQ-1 and #230 state match the
   accepted feature-local DESIGN artifacts.
2. Tighten proof-status wording in the brief and C4 intro so Slice 00's remaining
   integration gaps stay visible.
3. Update the feature-delta handoff note to say back-propagation is complete.
4. Optionally reclassify or status-clarify `OUT-MTLS-COMPOSED-PROXY-SKELETON`.
