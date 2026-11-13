# Independent DELIVER final evidence review

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Iteration | 2 — final; iteration 1 disposition recorded below |
| Review scope | Final integration report, finalization and evolution drafts, approved roadmap and feature delta, step review verdicts, D11 mapping review, native product capture, website verification, mutation-gate disposition, and retained command receipts |
| Evidence source | Final integration report committed at `2681befe6b3139a07f7a86316e211a26f8dab6e2`; exact closure selection ran at source `c2317cbcb4abc633de648f1c856a273e24a96fff`; final D11 model/test-support commit `a793389a4f7e967fbc12ba4c568509d3f46a35db` |
| Review type | Independent evidence and closure audit; no production or test implementation review |
| Reviewer changes | This Markdown artifact only; no commit created |
| Verdict | **APPROVED — iteration 2; F-01 closed with residual timing sensitivity recorded** |

## Audit result

The evidence record is careful about source identity and limits. It does not
present the 4,643-test diagnostic as green, claim that E13 proves the new D15
default, or claim mutation results. Iteration 1 held closure on an explicit
05-06 acceptance criterion: after excluding only the three named T-5 and
D-22 external failure identities, every other existing `overdrive-init`
guest-flow and beacon test must pass. The original serial run at `dca2350aa`
returned 209/210, and the focused Fast Open replay passed 1/1. The integration
owner then repeated the same complete serial selection after fresh GuestKit
preparation; that run passed 210/210 with three existing skips. The exact
required selection is now established for that recorded current-source run.
The original failure remains a real retained result with unresolved cause;
the later pass neither erases it nor proves it harmless.

## Evidence reviewed

| Claim | Evidence and audit disposition |
|---|---|
| Step and DES completion | All 17 step review artifacts end in `APPROVED`. The final integration report records `des-verify-integrity` exit 0 for all 17 and a repeat at the D11 source. I found no later DES phase event attributed to a final-gate agent. The separate D11 mapping correction has its own approved DISTILL review. |
| D15 native product journey | The retained E08 capture uses the built default-feature `overdrive` CLI and checked-in `zero-probes*.toml` examples. The positive output reports `Stable` in 1,363 ms with `guest listeners tcp:18081` passing. The negative output records six allocation rows, a missing `tcp:18998` after each 60-second startup window, and `StartupProbeFailed` on re-submission of the same checked-in spec. The command/session logs and `workload describe` output substantiate these claims. |
| Cleanup scope | The positive capture records zero VM, probe-ownership, network, cgroup, run-directory, mount, loop, and preparation deltas. The negative capture records zero VM, network, cgroup, run-directory, mount, loop, preparation, and serve deltas; it does **not** measure a probe-ownership delta. The finalization and evolution drafts preserve that distinction. |
| Historical E13 | E13 remains a SHA-pinned record for the former inferred-TCP claim. It is not cited as proof of the D15 default, and no expectation runner was invoked to rewrite it. The one-time E08 command/output capture is an appropriate retained evidence record under `verification.md`; no recurring expectation runner or new expectation ID is needed. |
| D11 stream-snapshot mapping | The original failing seed `0x43d4164d` passes 9/9 after the test/model mapping correction; the second seed passes 9/9 and the randomized `0x94ef88cb` selection passes 1/1. The reviewed record also shows 84/84 formal checks, 53/53 Quint typechecks, evidence verification and conformance lint. The independent review confirms the correction is confined to the existing observation port/test model and does not change production or public API. Its source-parity limit is stated: these focused results came from the documented isolated checkout, and are not represented as an all-workspace run at the D11 commit. |
| Scoped acceptance and static checks | The prepared control-plane acceptance binary passed 467/467 with three existing skips. The report separately records schema evolution 48/48, current model evidence, all 11 changed-crate formatting, scoped strict Clippy, and the D11 control-plane strict Clippy result. Whole-workspace format and Clippy failures remain attributed to unchanged `xtask` and `overdrive-host` files. |
| Required guest-flow/beacon serial selection | The original run at `dca2350aa` remains **209 passed, 1 failed, 3 skipped** of 210; the Fast Open child-correlation row missed the unchanged one-second bound. The exact isolated replay passed 1/1. A fresh GuestKit preparation at source `c2317cbcb` then supported the **identical complete serial selection**, which passed **210/210 with 3 existing skipped cases** in 607.180 seconds. The command profile, one-worker setting, three exact T-5/D-22 exclusions, source/test code, and one-second assertion were unchanged. Both receipts remain; no cause or expanded waiver is claimed. |
| Broad native diagnostic | The report records the run at `dca2350aa` as **failed**: 4,512 passed, 129 failed, two timed out, and 27 skipped, exit 100. It retains identities, distinguishes focused oracle corrections from unresolved unrelated failures, and does not claim a workspace-green result or waive failures by package grouping. |
| Website and publication | The website record and independent review are approved. Changed-content discovery, links, build, typecheck, and lint checks are recorded as passing. The existing search top-hit assertion is recorded as failing identically on untouched HEAD; assertions after that failure are not claimed as passing. No deployment or publication occurred. |
| Mutation gate | The roadmap records the user's exact instruction, `no mutation testing is required`, with `required: false` and `status: not_required_by_user`. No final-gate mutant run or kill rate is claimed. This waives the gate and its D17 mutation witnesses only; the approved production invariants remain in force. I did not run mutation testing. |

## F-01 — Iteration 1 blocking finding — required guest-flow/beacon serial selection has an unwaived failure

**Requirement:** Roadmap step `05-06` requires the existing
`overdrive-init` guest-flow and beacon tests to stay green except for the
three exact identities covering the user-approved T-5 and D-22 failure
modes. The 05-06 verification record says a different assertion, failure
mode, or test is outside that disposition.

**Observed result:** The final integration report records the serial native
selection at source `dca2350aa` as **209 passed, 1 failed, 3 skipped** of 210
selected tests, after excluding only the named T-5 and D-22 identities. The
failed identity is
`overdrive-init::integration::in_guest_lane::tcp_correlation_preserves_identity_and_cleanup_in_a_real_guest`.
Its guest assertion failed because the direct Fast Open child-correlation
row did not appear within one second (`crates/overdrive-init/src/guest_flow/real_kernel.rs:519`). The retained receipt is
`/private/tmp/service-readiness-final-guest-controls.log`, Metal run
`623b9cf022d67db8c20cbe99`, Nextest run
`a012615c-d52f-45ea-a8d1-30940afee717`. The exact focused replay later passed
1/1 in Metal run `011530b271c2abac7245b6f8`; this does not change the serial
selection's failed result.

**Scope and interpretation:** Fast Open is not one of the three authorized
external identities. The prior 05-06 record documents timing sensitivity for
this same one-second assertion, but neither the prior passing serial run nor
the focused replay establishes why the current serial run failed. This audit
does not infer a production defect, attribute the failure to the feature, or
prescribe a code change. It also cannot call the required selection green or
extend the existing waiver to this identity.

**Iteration 1 disposition:** Open. The required selection had one nonpassing
identity outside the approved external exceptions. The exact replay was not a
green result for the failed serial selection. Finalization remained pending;
no cause or code remediation was inferred.

## Iteration 2 — F-01 closure re-review

The integration owner reran the exact required serial selection after fresh
GuestKit preparation at source
`c2317cbcb4abc633de648f1c856a273e24a96fff`. The preparation used the same
isolated checkout and staged 4,437 runtime inputs for kernel
`7.0.0-38-generic`, target `x86_64-unknown-linux-musl` (Metal run
`2b68628bba6e7102be6ab169`). The retained foreground command was:

```text
cargo xtask metal run -- env NEXTEST_PROFILE=ci cargo nextest run -p overdrive-init --features integration-tests,kvm-tests -E 'not (test(quint_fwd_pair_pair_conf_guest) | test(a_burst_of_host_aborts_resets_every_connection_in_a_real_guest) | test(a_burst_of_host_aborts_resets_every_application_connection))' --no-fail-fast -j 1
```

The complete durable receipt reports 210 tests across the `overdrive-init`
library and integration binaries: **210 passed, 0 failed, 3 skipped**, in
607.180 seconds, exit 0. Metal run `bf34c3e577b5b52adf0cebef`; Nextest run
`6e93601c-55c2-4006-b15b-0767c5725164`; receipt
`/private/tmp/service-readiness-final-guest-controls-closure-durable.log`.
The command log at
`/private/tmp/service-readiness-final-guest-controls-closure.log` retains the
exact selection and profile. The original 209/210 failure and exact focused
1/1 pass remain intact as separate evidence.

The 210/210 run establishes the step's existing serial guest-flow/beacon
criterion for this recorded current-source execution. The original
one-second Fast Open failure remains outside the two authorized exceptions;
the pass does not explain or eliminate that failure mode. A separate
read-only diagnosis records that the failure's cause is undetermined and
that source-level comparison does not prove identity of every compiled guest
artifact or concurrent host condition. No test, assertion, timeout bound,
selection, production source, or waiver changed. This review does not declare
the failure harmless or infer a causal fix, and it makes no workspace-green
claim.

**Iteration 2 disposition:** F-01 closed for closure gating based on the exact
required serial selection passing at the current source after same-source
preparation. Residual timing sensitivity remains recorded, not waived.

## Verification limits and reviewer actions

I inspected the retained outputs and reports named above; I did not run tests,
native commands, mutation testing, or expectation runners. The current
workspace has unrelated dirty `AGENTS.md`, `netns-density-295` files,
`specs/quint/guest-flow-owner/fwd_pair.qnt`, and
`spike-scratch/step-05-07-sigkill/` work. I did not modify or stage them.

No other evidence finding remains. The final integration report's claims and
limits are supported by the retained evidence; F-01 is closed with the
iteration-2 result above. The prior nonpassing and broad diagnostic receipts
remain failures, and this audit does not claim that the workspace is green.
**Final verdict: APPROVED — iteration 2.**
