# DESIGN Wave Review - built-in-ca-operator-composition

**Reviewer**: `nw-solution-architect-reviewer` stance  
**Date**: 2026-06-09  
**Iteration**: rev 2 follow-up  
**Verdict**: **CONDITIONALLY APPROVED - architecture accepted; two handoff cleanups before EDD satisfaction / DELIVER closeout**

**Scope**: `docs/feature/built-in-ca-operator-composition/feature-delta.md`,
`design/wave-decisions.md`, `design/c4-diagrams.md`, ADR-0063 amendment,
ADR-0067 rev 6 amendment, `docs/product/architecture/brief.md` extension,
`.claude/rules/workflows.md` correction, and E03/O05 expectation docs.

## Verdict

The current artifact set resolves the prior blocking review findings.

- Internal workload-SVID near-expiry reissue is consistently framed in the
  feature docs as a live `Action::IssueSvid` with `"rotate-svid"` correlation,
  not as a `StartWorkflow(cert_rotation)` branch.
- The production boot move is properly scoped as composition: replace the
  ephemeral `RcgenCa` boot block with the already-implemented `boot_ca` +
  `bootstrap_node_intermediate` path, preserving KEK and envelope probes.
- The operator surface remains additive: `AllocStatusResponse.issued_certificates`
  exposes current cert metadata only, with no cert bytes or keys.
- The E03/O05 split is now explicit in both the feature delta and short
  wave-decisions handoff: O05 is the status summary render; E03 is the separate
  exported-PEM `openssl verify` proof.

The design is ready for DISTILL / DELIVER, with two non-blocking cleanups called
out below. Do not carry forward the previous "revisions needed" review state.

### Blocking-Issue Count

| Category | Issues |
|---|---:|
| Critical | 0 |
| High | 0 |
| Medium | 2 |
| Low | 0 |

## Findings

### Medium: E03 runner still enforces only two of the three documented sub-claims

**Dimension**: EDD testability / evidence completeness  
**Location**: `verification/expectations/E03-ca-full-chain-verifies/runner.sh:40-54`; contrast `verification/expectations/E03-ca-full-chain-verifies/README.md:41-54`, `docs/feature/built-in-ca-operator-composition/feature-delta.md:262-272`, `docs/feature/built-in-ca-operator-composition/design/wave-decisions.md:86-92`

The design correctly states that E03 requires three checks: full chain verifies,
leaf profile is correct, and the pathLen=0 negative anchor fails for a further-CA
chain. The runner currently checks only the positive chain and leaf-profile
claims, then exits.

This does not invalidate the design because the feature delta explicitly assigns
the runner update to Slice 3. It is still a handoff risk: a DELIVER agent could
wire the PEM export, see the runner return success, and treat E03 as complete
without the negative anchor.

**Recommendation**: In Slice 3, update the E03 runner before any `satisfied`
status is accepted. Either export the further-CA PEM from
`rcgen_intermediate_cannot_sign_a_further_ca_path_len_enforced` under the same
env gate and assert `openssl verify` fails, or capture that test's own failing
verification evidence. The different-fox review must reject E03 evidence if
sub-claim 3 is missing.

### Medium: older ADR / brief references still overload `#40` across internal SVID and external/public-trust rotation

**Dimension**: Cross-artifact consistency / issue-boundary clarity  
**Location**: `docs/product/architecture/adr-0063-built-in-ca-port-trait-and-root-key-protection.md:274-282`, `:581-589`, `:607-609`; `docs/product/architecture/brief.md:4363-4365`, `:5358-5370`, `:5387`, `:5444-5452`, `:5459`, `:5462`

The current feature docs, `.claude/rules/workflows.md`, ADR-0067 rev 6, and the
new brief extension all make the load-bearing distinction: internal workload-SVID
near-expiry reissue is closed here as `Action::IssueSvid`; only external ACME /
public-trust or root-rotation work remains workflow-shaped.

Some older ADR-0063 and workflow-result sections still say "#40" or
"future #40 rotation workflow" without that qualifier. Most of these are
historical or adjacent sections, not the primary handoff path, but the issue
number reuse can still mislead a reader into thinking internal SVID rotation
depends on #39/workflows.

**Recommendation**: Rewrite those references to either:

- "external-ACME / public-trust rotation" or "root-CA rotation" when that is the
  actual future workflow-shaped concern, or
- "historical, superseded #40 framing" when preserving provenance.

Avoid unqualified "#40 cert-rotation" in current prose after this feature, since
the internal SVID near-expiry scope of #40 is closed by this design.

## Resolved Prior Findings

- **Stale #40 workflow/gate model**: resolved in the active feature handoff.
  `feature-delta.md:55-63`, `:128-136`, `:171-190`; `wave-decisions.md:8-17`;
  ADR-0067 rev 6 and `.claude/rules/workflows.md` all state the same action-not-
  workflow model.
- **E03/O05 collapse**: resolved in the short handoff. `wave-decisions.md:82-97`
  now says the `issued_certificates` render captures O05 only and E03 is the
  exported-PEM `openssl verify` path.
- **#215/operator-surface status**: resolved in the feature delta, ADR-0063
  amendment, ADR-0067 rev 6 boundary, and brief extension. #215 is no longer
  described as blocked on #35 in the active handoff.
- **`LocalStore` terminology in this feature**: resolved. The driven-port table
  uses `LocalIntentStore (redb)` for the intent adapter.

## Strengths

- Reuse posture is strong: `boot_ca`, `bootstrap_node_intermediate`,
  `Action::IssueSvid`, the existing issue executor, `issued_certificate_rows()`,
  and `SpiffeId::for_allocation()` are reused rather than duplicated.
- The workflow boundary is now coherent. A single internal mint-and-swap belongs
  behind the existing action executor, not a durable workflow.
- The boot-side Earned-Trust invariant is explicit: KEK resolve and envelope
  decrypt happen before production use, with typed `CaBootError` propagation.
- The O05/E03 evidence split avoids leaking cert bytes into the operator status
  surface while still preserving a real `openssl verify` proof path.

## Approval Status

`conditionally_approved`

No new architecture iteration is needed. Before final DELIVER closeout, tighten
the E03 runner to enforce sub-claim 3 and clean the remaining unqualified `#40`
references so downstream agents see one current issue boundary.
