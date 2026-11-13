# DELIVER finalization record — service allocation readiness and ending

**Finalized:** 2026-11-13. **Status:** DELIVER complete. The final scoped
receipts and [independent evidence audit](review-final-evidence.md) are approved;
historical failed diagnostics and their limits remain part of this record.

## Completed implementation and review

The [approved roadmap](roadmap.json) has 17 steps across five phases. All 17
have completed DES traces and on-disk final `APPROVED` reviews
([01-01](review-01-01.md), [01-02](review-01-02.md),
[02-01](review-02-01.md), [02-02](review-02-02.md),
[02-03](review-02-03.md), [02-04](review-02-04.md),
[03-01](review-03-01.md), [03-02](review-03-02.md),
[04-01](review-04-01.md), [04-02](review-04-02.md),
[05-01](review-05-01.md), [05-02](review-05-02.md),
[05-03](review-05-03.md), [05-04](review-05-04.md),
[05-05](review-05-05.md), [05-06](review-05-06.md),
[05-07](review-05-07.md)). `des-verify-integrity` exited 0 for all 17 at
the recorded final-integration source. No final-gate correction rewrote a DES
phase event.

The later, bounded final-gate corrections remain linked to their owners:

| Scope | Source and independent disposition |
|---|---|
| 05-07 native support and D15 schema | Implementation `229edec40`; audit/comments `62917d18`; schema description/regeneration `d727375c` and `d7386262`. [Review 05-07](review-05-07.md) iteration 3 **APPROVED** in `67b7a8551`. |
| 05-02 lint and test support | Async fixture `493e450d`; exact scanner `b23de5ca` plus audit `9b387cb`; function-local Contract Shape lint allowances `45086aec`/`b0881ff6`. [Review 05-02](review-05-02.md) iteration 3 **APPROVED** in `db959912c`; full DST lint and scoped strict Clippy passed. |
| 03-02 private broker adapter | Private `cfg` correction `ff29358a8`/`32a60dd17`. [Review 03-02](review-03-02.md) iteration 3 **APPROVED** in `fdcb73a64`; D17 runtime behavior is unchanged and five wake controls passed. |
| D7/D9/D10/D12 test oracles | Deadline text `530b64ae`; fixture corrections `e4635dd3`; backend 12/12 and CLI scoped lint allowance `5cce053c`; handler Contract Shape `6b7d2680`. [DISTILL oracle review](../distill/review-final-oracle-alignment.md) iteration 2 **APPROVED**. This verdict excludes the separate D11 mapping. |
| D11 private conformance mapping | Model and test driver correction `a793389a4`; [DISTILL record](../distill/final-d11-mapping-alignment.md) and [independent review](../distill/review-final-d11-mapping-alignment.md) iteration 1 **APPROVED** in `ac6c57d14`. Focused Metal: original seed `0x43d4164d` 9/9, second seed `0xd1acbdab` 9/9, randomized seed `0x94ef88cb` 1/1. Canonical formal 84/84 and typed error oracle passed at the reviewed isolated source. These are focused source-parity results, not a whole-workspace green claim. |

## Product, website and verification evidence

The [05-07 native receipt](step-05-07-verification.md) and its approved review
record the built default-feature `overdrive` product path using the checked-in
`examples/service-kind-vm-workloads/zero-probes*.toml` specs. A listening
Service reached `Stable` in 1,363 ms with `guest listeners tcp:18081` passing.
The never-listening Service produced six allocation rows and a terminal
`StartupProbeFailed` naming missing `tcp:18998` after the identical spec was
redeployed. Both owned cleanup captures report zero on their respective
measured surfaces. Only the positive capture measured probe-ownership delta.
The [historical E13 receipt](../../../../verification/expectations/E13-vm-service-inferred-tcp-startup/README.md)
remains SHA-pinned and unchanged; it is not current-default evidence.

The five operator pages—[architecture](../../../../website/content/docs/concepts/architecture.mdx),
[microVMs](../../../../website/content/docs/concepts/microvms.mdx),
[reconcilers](../../../../website/content/docs/concepts/reconcilers.mdx),
[deploy guide](../../../../website/content/docs/how-to/deploy-a-workload.mdx)
and [CLI reference](../../../../website/content/docs/reference/cli.mdx)—and the
[engineering post](../../../../website/content/blog/when-a-microvm-service-is-ready.mdx)
were committed in `5cd87c7a` and revised in `edd592233`. The
[website validation record](website-verification.md) records passing changed
content discovery, links, build, typecheck and lint checks. Its existing
search-ranking assertion fails identically on untouched HEAD; the stopped
scripts' later assertions are not claimed as passing. The independent
[website review](review-website.md) iteration 2 is **APPROVED**. No deployment
or publication occurred.

The [final integration report](final-integration-verification.md) records the
diagnostic full native Nextest result at `dca2350aa`: 4,643 run across 93
binaries, 4,512 passed, 129 failed, two timed out, 27 skipped, exit 100.
Current-feature failures were isolated and routed for bounded correction; the
other nonpasses retain their exact identities and limits in that report. The
first required guest-flow/beacon serial selection returned 209 passed, one
failed and three skipped; the exact Fast Open replay passed 1/1. The later
**identical complete serial selection** passed 210/210 with three existing
skips at source `c2317cbcb`, after fresh same-source GuestKit preparation and
only the three named T-5/D-22 identities excluded.
The original one-second Fast Open failure remains a failed receipt with
undetermined cause and residual timing sensitivity; no test bound, selection,
or exception changed. The complete control-plane acceptance binary passed
467/467 with three existing skips at source `6b7d26806`, and all 11 changed
crates passed scoped strict Clippy and formatting at their reported sources.
Whole-workspace formatting still fails in an unchanged `xtask` file. Strict
whole-workspace Clippy still fails at unchanged-from-`origin/main`
`overdrive-host/tests/integration/vm_host_state_equivalence.rs:172` on its
required Contract Shape line versus `doc_markdown`. None of these results is
called globally green or silently waived.

The user's exact instruction, **“no mutation testing is required,”** is
recorded by `a84efd8e3` in the roadmap final gate: `required: false`,
`status: not_required_by_user`, including the former D17 mutation witnesses.
No final-gate mutants ran and no kill rate is claimed. This changes the final mutation
gate only; D17 and the other production invariants remain binding.

## Final gate and residual limits

The [final integration report](final-integration-verification.md), updated in
`2681befe6`, retains the exact serial closure replay and its same-source
GuestKit prerequisite. The [independent final evidence audit](review-final-evidence.md),
committed in `c431ca57c`, is **APPROVED at iteration 2**. Its F-01 closed
because the required unchanged serial selection passed at `c2317cbcb`, with
no enlarged exception, weakened one-second assertion, or inferred causal fix.
The integration report's header records the pre-audit pending state at its
commit; the later approved audit supplies the final review disposition.
All 17 step reviews, the bounded remediation reviews, the D11 and oracle
DISTILL reviews, and the website review are approved. The broad diagnostic,
initial serial failure, unrelated whole-workspace Clippy/format failures, and
pre-existing website search assertion remain recorded as failures at their
named sources. No workspace-wide green result is claimed.

The feature workspace, accepted ADRs, product examples and canonical
verification catalogue remain at their existing homes. No workflow or review
artifact was archived or deleted, and the website was neither deployed nor
published. DELIVER finalization does not authorize those later actions.
