# Evolution — service allocation readiness and ending

**Feature:** `service-allocation-readiness-and-ending` (GH #318, GH #334).
**Record prepared:** 2026-11-13. **Status:** verification pending; **not finalized**.
The [feature contract](../feature/service-allocation-readiness-and-ending/feature-delta.md),
[approved roadmap](../feature/service-allocation-readiness-and-ending/deliver/roadmap.json)
and [finalization record](../feature/service-allocation-readiness-and-ending/deliver/finalization.md)
are the detailed authorities; this record will become the evolution summary
only after the pending receipts receive independent approval.

## Delivered outcome and decisions

The 17 implemented and independently approved roadmap steps end probe
supervision on a VM allocation's natural exit, keep `WorkloadLifecycle` as the
single restart and give-up owner, stop a startup-failing attempt before
replacement, begin liveness at committed `Stable`, and withhold a startup-
probed allocation from Service backends until its own startup passes. A Service
deploy stream reports a recorded current-attempt verdict before waiting and
uses the derived progress deadline. The default Service startup gate waits for
the guest to report every declared TCP and UDP listener; an explicitly empty
startup list remains the opt-out. The accepted D1–D17 interface and error
contracts remain in the [feature delta](../feature/service-allocation-readiness-and-ending/feature-delta.md).

The native default-feature `serve`/`deploy` capture reached `Stable` in
1,363 ms with `tcp:18081` reported. The contrasting never-listening Service
recorded six allocation attempts and a terminal `StartupProbeFailed` naming
missing `tcp:18998` on identical redeploy. Cleanup was zero on each capture's
respective measured surfaces; the negative capture did not measure a probe
ownership delta. The [05-07 receipt](../feature/service-allocation-readiness-and-ending/deliver/step-05-07-verification.md)
and [independent review](../feature/service-allocation-readiness-and-ending/deliver/review-05-07.md)
hold the source, commands and observations. Historical
[E13](../../verification/expectations/E13-vm-service-inferred-tcp-startup/README.md)
remains unchanged at its former inferred-TCP SHA and is not proof of this new
default.

## Delivery and validation disposition

All 17 DES traces verified and all 17 step reviews end `APPROVED`. Bounded
final-gate corrections to 05-07, 05-02, 03-02 and D7/D9/D10/D12 test oracles
have their separate approved review iterations and commits in the
[finalization record](../feature/service-allocation-readiness-and-ending/deliver/finalization.md).
The private D11 `stream_snapshot` poll/subscribe mapping correction
`a793389a4` is [independently approved](../feature/service-allocation-readiness-and-ending/distill/review-final-d11-mapping-alignment.md)
at iteration 1. Its focused isolated-source evidence passed 9/9 at the
original `0x43d4164d` seed, 9/9 at a second seed, 1/1 at a third randomized
seed, and 84/84 canonical formal checks. It does not establish a broad
affected-suite or later-source result. Latest-source scoped integration
rechecks and an independent final evidence audit remain pending.

The broad native workspace diagnostic at `dca2350aa` exited 100: 4,512
passed, 129 failed, two timed out and 27 skipped across 4,643 runs. The
serial required guest-flow/beacon controls recorded 209 passed, one Fast Open
failure and three skipped; the exact failed cell later passed 1/1 in isolation,
without making the serial run green. The [integration report](../feature/service-allocation-readiness-and-ending/deliver/final-integration-verification.md)
retains the failed identities, routed current-feature work and remaining
limits. Whole-workspace formatting and strict Clippy also have unrelated
unchanged-file failures; feature-scoped results are recorded separately.

The user's exact **“no mutation testing is required”** instruction changed
the roadmap's final mutation gate to `not_required_by_user`, including its
former D17 witnesses. No final-gate mutation run or score is claimed, and production
invariants remain binding. Five operator documentation pages and the
[engineering post](../../website/content/blog/when-a-microvm-service-is-ready.mdx)
are checked in. The [website record](../feature/service-allocation-readiness-and-ending/deliver/website-verification.md)
and [independent review](../feature/service-allocation-readiness-and-ending/deliver/review-website.md)
record passing changed-content checks and an unchanged-HEAD search-ranking
failure. The website was not deployed or published.

The accepted ADRs, feature workspace, review history, product examples and
SHA-pinned verification catalogue remain at their existing repository homes.
No finalization migration or cleanup has been performed while verification is
pending.
