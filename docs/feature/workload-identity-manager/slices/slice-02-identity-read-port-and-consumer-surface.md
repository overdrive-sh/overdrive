# Slice 02 — Shared read surface behind the `IdentityRead` port

> **DESIGN RESOLVED THE OPEN SURFACES — implement ADR-0067 (rev 2), NOT the
> "DESIGN call" text below.** The pinned signatures: `IdentityRead { svid_for(&
> AllocationId) -> Option<SvidMaterial>; current_bundle() -> Option<TrustBundle> }`
> with 5 behaviour-pinning rustdoc clauses (ADR-0067 D7). The trust-bundle currency
> mechanism is RESOLVED as **HYDRATED into `IdentityMgr`** (set at boot, refreshed
> by the issue executor, pushed by #40 via `set_bundle`; zero CA I/O on the read hot
> path — ADR-0067 D6) — NOT "pull-on-demand vs hydrated, DESIGN's call." Implement
> ADR-0067 rev 2 D6/D7, not the Open-Questions "DESIGN call" wording below.

**Job**: J-SEC-002 | **Feature**: workload-identity-manager (GH #35) | **Story**: US-WIM-02
**Release**: Release 1 (first enhancement past the walking skeleton)

## Goal (one sentence)

Expose the held SVID + current trust bundle to the dataplane consumers
(sockops/gateway/telemetry) through an `IdentityRead` port trait — sync,
in-process getters that never re-issue per read — with a `SimIdentityRead`
double, so the unbuilt consumers (#26 etc.) have a sound, low-latency, mockable
seam to read identity from.

## IN scope

- `IdentityRead` port trait in `overdrive-core` (core class), signatures pinned by
  ADR-0067 D7: `svid_for(&AllocationId) -> Option<SvidMaterial>` + `current_bundle()
  -> Option<TrustBundle>`. The trait docstring pins **5 observable clauses** every
  adapter MUST honor: (1) a read NEVER triggers issuance (no `Ca::issue_svid` on the
  read path — O3); (2) a read NEVER mutates; (3) `None` = explicit absence; (4)
  returns owned clones (no lock held after the read); (5) post-`DropSvid(alloc)`,
  `svid_for(alloc) == None`.
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
- Trust-bundle currency wiring — RESOLVED **HYDRATED** (ADR-0067 D6):
  `current_bundle()` reads the bundle held in `IdentityMgr` (set at boot via
  `IdentityMgr::new(Some(Ca::trust_bundle()))`, refreshed by the issue executor
  after `issue_and_audit` via `identity.set_bundle(ca.trust_bundle()?)`). ZERO CA
  I/O on the read hot path; `set_bundle` is #40's push seam. (NOT pull-on-demand.)

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
- [ ] A read for a held allocation returns the current `SvidMaterial` + `TrustBundle` **without re-issuing** — NO `issue_svid` on the read path; the SVID is served from the held map and the bundle is served from the **hydrated** `IdentityMgr` (ADR-0067 D6 — ZERO CA I/O on the read hot path, the O3 guarantee).
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
the workspace. Both previously "DESIGN-pinned" surfaces (the getter signatures +
the trust-bundle currency mechanism) are RESOLVED in ADR-0067 rev 2 (D7 signatures +
5 clauses; D6 HYDRATED bundle) before crafting.

## Taste-test note

One additive surface (the read port) on the held store Slice 01 built. Production
relevance via the equivalence test + a test consumer driving the port, with
`openssl verify` (TEST tier) on the leaf the getter returns as the executable
proof (the operator `alloc status` render is #215's, blocked on #35). Disproves a
real pre-commitment (an in-process, no-re-issue read surface for the consumers).
Distinct from every other slice (the READ tier — getters + sim double, no
lifecycle, no durability). Carries a value story (US-WIM-02) — NOT infra-only.
