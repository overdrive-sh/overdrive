# DISTILL test contract correction — Service timeout render

## Contract and failure

Approved D10 item 7 in `feature-delta.md` requires the CLI `Timeout` reason to say ``workload did not become Stable within {after_seconds}s; the platform may still be restarting it (see `overdrive workload describe`)``. The independent step 04-01 review approved the renderer's D10 wording. `format_service_failed_block` renders that exact reason at `crates/overdrive-cli/src/render.rs:1562`.

The retained unmutated baseline at `dca2350aad3a9dc5994e1788605fb4903fa8c127` failed `overdrive-cli::acceptance::render_pure_fns::failed_block_reason_text_is_variant_specific`: its `Timeout { after_seconds: 90 }` case still required the superseded `workload did not converge within 90s` substring. The baseline result is recorded in `../deliver/mutation/mutation-report.md`; the correction does not infer a production defect from that failure.

## Correction and verification

Only that existing Timeout case now requires the complete D10 sentence for 90 seconds. Every other variant case and the loop assertion are unchanged. The transitioned test also carries its own `/// CONTRACT_SHAPE: pure-function.` declaration. No new test identity, production API, renderer change or mutation configuration was introduced.

| Evidence | Result |
|---|---|
| Source base for the original failure | `dca2350aad3a9dc5994e1788605fb4903fa8c127`, as retained in the final DELIVER baseline report |
| Isolated correction worktree source | `edd592233` before the test edit |
| `cargo xtask metal run -- env OVERDRIVE_BPF_NATIVE=1 cargo xtask bpf-build` | Passed; run `c5f566b213550099ba354128` prepared the isolated Metal checkout |
| `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests --test acceptance -E 'test(failed_block_reason_text_is_variant_specific)'` | Passed, 1/1 selected; run `d519dbff5f08368f763f3260` |
| `git diff --check` | Passed |

The user's subsequent waiver removes the final mutation gate; no mutation run was made for this correction.
