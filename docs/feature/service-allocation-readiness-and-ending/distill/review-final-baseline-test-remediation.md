# Independent DISTILL review — final baseline Timeout render assertion

## Metadata and verdict

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Scope | Existing `overdrive-cli::acceptance::render_pure_fns::failed_block_reason_text_is_variant_specific` assertion for `ServiceFailureReason::Timeout { after_seconds: 90 }` |
| Remediation commit | `530b64ae6b5be7bf7c7011c96a44962eb0ef25f1` |
| Authority | Approved feature-delta D10 item 7 and the final `APPROVED` verdict in DELIVER `review-04-01.md` |
| Verdict | **APPROVED** — bounded DISTILL test contract correction |

## Contract and reproduced mismatch

D10 item 7 specifies the Timeout reason as ``workload did not become Stable within {after_seconds}s; the platform may still be restarting it (see `overdrive workload describe`)``. The production `format_service_failed_block` Timeout arm renders that sentence. The retained unmutated baseline log (`.context/service-allocation-readiness-and-ending-mutation/baseline-nextest-retry.log`) shows this exact test failing on `Timeout { after_seconds: 90 }`: it expected `workload did not converge within 90s`, while the actual rendered reason contained the approved D10 sentence. This is an obsolete test oracle, not a production renderer failure.

The commit changes only that Timeout case's expected substring to the complete D10 sentence with `90s`. The existing `rendered.contains(expected)` assertion and every other reason variant's expected substring remain unchanged. The new expected string checks the Stable state, 90-second value, possible platform restart, and `overdrive workload describe` hint together; it does not relax the Timeout assertion. The transitioned test has its own exact `/// CONTRACT_SHAPE: pure-function.` declaration. No test identity was added or removed, and no renderer, production API, error variant, or mutation configuration changed.

## Verification and scope

| Evidence | Result |
|---|---|
| Focused Metal run | `final-baseline-test-remediation.md` identifies the selection as `cargo nextest run -p overdrive-cli --features integration-tests --test acceptance -E 'test(failed_block_reason_text_is_variant_specific)'`. I independently collected its durable run `d519dbff5f08368f763f3260` with `cargo xtask metal collect`; the record confirms exit code 0, **1 passed, 119 skipped**. This is evidence for the focused selection, not a claim about the full suite. |
| Commit diff | Exactly two files: the existing CLI acceptance test and `distill/final-baseline-test-remediation.md`. The latter records the contract, original failure, correction, and focused Metal run. |
| Diff integrity | `git show --check 530b64ae6b5be7bf7c7011c96a44962eb0ef25f1` passed. |
| Attribution | Git author remains Marcus Schack Abildskov; the commit message contains exactly `Co-Authored-By: Codex <codex@openai.com>` and no Claude or Anthropic attribution. |
| Worktree | Pre-existing dirty `AGENTS.md`, netns design/roadmap files, `fwd_pair.qnt`, and the untracked spike remain untouched. |

The user waived the final mutation gate, so this bounded review requires no mutation run or D17 mutant witness. The previously approved 17 DELIVER steps and documentation review are outside this test-only correction's verdict.

## Findings and disposition

No blocking finding. The obsolete Timeout substring was replaced by the exact accepted D10 wording; the remaining per-variant assertions and loop are intact. No remediation remains for this bounded change.

**Final verdict: APPROVED.** The corrected test now passes in the focused Metal selection and asserts the approved Timeout reason without weakening the other variant checks.
