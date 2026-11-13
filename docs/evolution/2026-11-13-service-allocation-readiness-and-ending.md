# Evolution — service allocation readiness and ending

**Feature:** `service-allocation-readiness-and-ending` (GH #318, GH #334).
**Finalized:** 2026-11-13. **Status:** DELIVER complete.
The [feature contract](../feature/service-allocation-readiness-and-ending/feature-delta.md),
[approved roadmap](../feature/service-allocation-readiness-and-ending/deliver/roadmap.json)
and [finalization record](../feature/service-allocation-readiness-and-ending/deliver/finalization.md)
are the detailed authorities. The
[independent final evidence audit](../feature/service-allocation-readiness-and-ending/deliver/review-final-evidence.md)
approved the final scoped receipts at iteration 2.

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
affected-suite or later-source result. The
[final integration report](../feature/service-allocation-readiness-and-ending/deliver/final-integration-verification.md)
records the complete control-plane acceptance binary passing 467/467 at
`6b7d26806` and the exact required guest-flow/beacon serial selection passing
210/210 with three existing skips at `c2317cbcb` after fresh same-source
GuestKit preparation and only the three named T-5/D-22 identities excluded.
The independent final evidence audit is **APPROVED at
iteration 2**; its F-01 closed on this exact serial replay.

The broad native workspace diagnostic at `dca2350aa` exited 100: 4,512
passed, 129 failed, two timed out and 27 skipped across 4,643 runs. The
first serial required guest-flow/beacon selection recorded 209 passed, one
Fast Open failure and three skipped; the exact failed cell later passed 1/1
in isolation. The original failure remains a failed receipt with undetermined
cause and unchanged one-second bound. The later complete serial pass does not
make the original or the broad diagnostic green. The integration report
retains the failed identities and source limits. Whole-workspace formatting
and strict Clippy also have unrelated unchanged-file failures; all 11 changed
crates passed scoped formatting and strict Clippy at their reported sources.

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
No finalization migration or cleanup removed those canonical records. This
finalization performed no website deployment, publication, push, or pull request.
