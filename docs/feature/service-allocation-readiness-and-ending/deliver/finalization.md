# DELIVER finalization record — service allocation readiness and ending

**Prepared:** 2026-11-13. **Status:** verification pending; **not finalized**.
This is a closeout draft. Pending results below require their actual receipts
and independent review before the status changes.

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
required serial guest-flow/beacon controls returned 209 passed, one failed and
three skipped. The failed Fast Open cell passed 1/1 on exact focused replay,
but the serial suite remains failed. Whole-workspace formatting fails on an
unchanged `xtask` file. Strict whole-workspace Clippy after feature corrections
still fails at unchanged-from-`origin/main`
`overdrive-host/tests/integration/vm_host_state_equivalence.rs:172` on the
required Contract Shape line versus `doc_markdown`; the latest scoped
feature-crate recheck is pending. None of these results is called globally green
or silently waived.

The user's exact instruction, **“no mutation testing is required,”** is
recorded by `a84efd8e3` in the roadmap final gate: `required: false`,
`status: not_required_by_user`, including the former D17 mutation witnesses.
No final-gate mutants ran and no kill rate is claimed. This changes the final mutation
gate only; D17 and the other production invariants remain binding.

## Pending closeout receipts

1. Resolve the private D11 randomized `stream_snapshot` model/test-driver
   poll/subscribe mapping at seed `0x43d4164d`, retain the named and randomized
   green results, and obtain its independent DISTILL review. No public contract
   defect has been established by the current failure.
2. Record the latest committed-source scoped integration rechecks and their
   exact selected identities, results and source SHA. The earlier broad
   diagnostic and focused historical passes do not stand in for this receipt.
3. Obtain an independent final evidence audit of the exact claims and limits,
   then record its verdict here and in the evolution record.

Until these receipts exist and are approved, the feature remains
**verification pending; not finalized**. This preparation does not archive or
remove the feature workspace, copy the canonical verification catalogue, or
publish the website.
