# Slice 02 — Shared read surface behind the `IdentityRead` port

**Job**: J-SEC-002 | **Feature**: workload-identity-manager (GH #35) | **Story**: US-WIM-02
**Release**: Release 1 (first enhancement past the walking skeleton)

## Goal (one sentence)

Expose the held SVID + current trust bundle to the dataplane consumers
(sockops/gateway/telemetry) through an `IdentityRead` port trait — sync,
in-process getters that never re-issue per read — with a `SimIdentityRead`
double, so the unbuilt consumers (#26 etc.) have a sound, low-latency, mockable
seam to read identity from.

## IN scope

- `IdentityRead` port trait in `overdrive-core` (core class): sync getters for
  the current SVID by `AllocationId` and the current `TrustBundle`. Trait
  docstring pins observable behaviour — including that a read NEVER triggers
  issuance and that an absent allocation is reported explicitly. (Exact
  signatures are a DESIGN call — recommended `svid_for(&AllocationId) ->
  Option<SvidMaterial>` + `current_bundle() -> TrustBundle`; feature-delta
  Open-Questions #2.)
- `IdentityMgr` implements `IdentityRead` by reading its held `BTreeMap` + the
  current bundle (read-lock, clone-out, drop guard — never holds the lock across
  `.await`, per `.claude/rules/development.md` § "Concurrency & async").
- `SimIdentityRead` sim double (`adapter-sim`) preloadable with fixture
  `SvidMaterial` + bundle, for consumer tests.
- A **test consumer/fixture** takes `Arc<dyn IdentityRead>` as a **required
  constructor parameter** (never defaulted to a production binding) — proving the
  port-trait discipline as a contract. The **production** consumers that take the
  port for real are deferred to their own features (see OUT scope).
- DST equivalence test driving the real `IdentityMgr` read path and
  `SimIdentityRead` through the same calls, asserting identical observable reads.
- Trust-bundle currency wiring (the mechanism — pull `Ca::trust_bundle()` on
  demand vs reconciler-hydrated — is DESIGN's call, feature-delta Open-Questions
  #5; this slice consumes whatever DESIGN pins).

## OUT scope

- The **production** dataplane consumers that USE the port — kernel-side
  sockops/kTLS mTLS (#26), the L7 gateway, the telemetry sink. This slice ships
  the *read port + its sim double + a test consumer that proves the read
  contract*, not the production consumers (building them here = single-cut
  violation; #26 owns the kernel surface).
- The `watch`/`broadcast` **push** read surface (notify-on-change, DIVERGE Option
  3) → future, a non-breaking change behind this port once a consumer demands it.
- Issue/hold/drop lifecycle → Slice 01 (this slice only READS what 01 holds).
- Restart-idempotence / the #40 seam → Slice 03.

## Learning hypothesis

- **Disproves if it fails**: "dataplane consumers can read the current SVID +
  trust bundle through an in-process `IdentityRead` port (sync getters, no
  re-issue per read), mockable via a sim double." If the read surface must
  re-issue per read, or cannot be exercised without the real CA, the O3
  read-latency outcome and the consumer seam (#26/gateway/telemetry) are
  compromised.
- **Confirms if it succeeds**: the consumers have a sound, low-latency, testable
  seam; the getter→`watch` push upgrade (Option 3) is a clean future change
  behind the same port.

## Acceptance criteria

- [ ] An `IdentityRead` port trait in `overdrive-core` exposes sync getters for the current SVID (by `AllocationId`) and the current trust bundle; the docstring pins that a read never triggers issuance.
- [ ] `IdentityMgr` implements `IdentityRead`; a **test consumer/fixture** takes `Arc<dyn IdentityRead>` as a required constructor parameter (never defaulted), proving the port-trait discipline as a contract. No lock held across `.await`. (Production consumer wiring — sockops #26 / gateway / telemetry — is deferred to those features, not an AC here.)
- [ ] A read for a held allocation returns the current `SvidMaterial` + `TrustBundle` **without re-issuing** — NO `issue_svid` on the read path; the SVID is served from the held map (the O3 guarantee). The **trust-bundle currency mechanism** (pull-on-demand via `Ca::trust_bundle()` vs hydrated into `IdentityMgr`) stays **DESIGN's call** (feature-delta Open-Questions #5) — a cheap bundle pull is permitted; re-issuing the SVID per read is not.
- [ ] A read for an absent allocation returns an explicit "absent" (e.g. `None`), not a stale or empty-but-present credential.
- [ ] A `SimIdentityRead` double exists; a DST equivalence test drives the real read path and the sim double through the same sequence and asserts identical observable reads.

## Dependencies

- Slice 01 (a populated `IdentityMgr` to read from).
- `SvidMaterial`, `TrustBundle`, `AllocationId` newtypes (exist).
- `overdrive-sim` for the `SimIdentityRead` double (exists as a crate).

## Effort estimate

~1 day (≤6h). Reference class: a port trait + host-backed impl + sim double + an
equivalence test is the standard project pattern (mirrors the `Ca` /
`ObservationStore` trait+adapter+equivalence shape). The new parts are the getter
signatures and the read-lock-clone-out discipline.

## Pre-slice SPIKE

Not needed — the port-trait+sim-double+equivalence pattern is well-trodden in
the workspace. The one DESIGN-pinned uncertainty (exact getter signatures + the
trust-bundle currency mechanism) is resolved by the architect before crafting.

## Taste-test note

One additive surface (the read port) on the held store Slice 01 built. Production
relevance via the equivalence test + a test consumer driving the port, with
`openssl verify` (TEST tier) on the leaf the getter returns as the executable
proof (the operator `alloc status` render is #215's, blocked on #35). Disproves a
real pre-commitment (an in-process, no-re-issue read surface for the consumers).
Distinct from every other slice (the READ tier — getters + sim double, no
lifecycle, no durability). Carries a value story (US-WIM-02) — NOT infra-only.
