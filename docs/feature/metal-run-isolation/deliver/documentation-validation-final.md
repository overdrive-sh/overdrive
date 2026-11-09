# Final documentation reconciliation and validation

## Scope and source

Date: 2026-11-09. Author: Codex. This record covers the final prose reconciliation of:

- [The Metal operator guide](../../../../website/content/docs/how-to/run-verification-on-metal.mdx).
- [The engineering post](../../../../website/content/blog/sharing-a-metal-box-without-sharing-test-state.mdx).
- The DELIVER `[REF]` disposition in [feature-delta.md](../feature-delta.md).

The author applied the repository [technical-writer skill](../../../../.claude/skills/technical-writer/SKILL.md), `.claude/rules/finalize.md` and `website/CLAUDE.md`. The final fact update follows evidence commit `d4d011b4fdd8e2954d5cf90f4da261702d88576f`. The final production correction is `060359bc1cf88c78252e9d789396821413b41d7e`.

The guide describes implemented commands and current limits. The post retains the rejected CID allocator, the pinned Linux local-mode vsock discovery, source and invocation ownership, shared ordinary versus exclusive maintenance admission, nextest ownership, and finalizer semantics. Its Quint account distinguishes the early 16 holds and two intentional hazard violations from later actual-owner conformance. That conformance reproduced a queued-controller-EOF production defect, then passed the exact failing seed, controller-loss controls and a fresh randomized seed after correction.

The final native account follows the [consolidated receipt](native-evidence/placement-final-complement-2026-11-09-114229Z/README.md). Worktree and admission drivers returned 0 at their recorded sources. Placement outcomes combine the original prefix, corrected marker checks, four existing execution-mode checks at frozen production source `060359bc`, and the final observer. The final observer returned 0 with zero forbidden physical deltas, zero owned processes and zero owned roots. Two raw finite RA expiry countdowns changed by one second under the existing approved comparison. The prose does not claim an uninterrupted full placement-driver pass at one commit.

Only the pre-existing owner-burst body was omitted. Its original fixture reproduced the same one-second, 78-of-1,000 assertion before this feature's implementation. It remains a failed body, with no product/test correction or fabricated pass. The source-scoped captures retain their historical failures. No throughput or speedup is claimed.

## Executed scoped checks

The final pass changes prose and metadata on existing routes. It adds no page, route, navigation entry, schema, rendering code, search code or export code. Following the orchestrator's explicit instruction, it repeats neither full builds nor the known unrelated search-ranking failure.

| Check | Actual result | Retained local record |
| --- | --- | --- |
| `bun run typecheck` after the completed prose | 0. Fumadocs regenerated its collections and TypeScript passed. | [typecheck-final.log](../../../../.context/metal-documentation-final-20261109/typecheck-final.log) |
| `bun run lint` during the initial reconciliation pass | 0. No website code changed afterward. | [lint.log](../../../../.context/metal-documentation-final-20261109/lint.log) |
| Authored links and anchors | All 12 page links resolve to local routes or exact retained Git objects. Both heading anchors resolve. This does not establish that an unpushed commit is publicly reachable. | [scoped-links-and-content-final.log](../../../../.context/metal-documentation-final-20261109/scoped-links-and-content-final.log) |
| Navigation and blog discovery | Guide appears exactly once in how-to navigation. Regenerated Fumadocs docs/blog imports and collection maps contain both pages. Blog metadata has the November 9 date and no `draft: true` override. | Same scoped log |
| Prose and contract scope | Both MDX files contain no semicolons. Every byte before the feature delta's DELIVER `[REF]` section matches HEAD. Existing disposition evidence links resolve. | Same scoped log |
| Whitespace | Scoped `git diff --check` passed. | Same scoped log |
| Preservation | All seven pre-existing unrelated dirty files have unchanged SHA-256 hashes. The other reviewer's `review-slice.md` is outside this author's ownership. | [Before](../../../../.context/metal-documentation-final-20261109/unrelated-before.json) and [after](../../../../.context/metal-documentation-final-20261109/unrelated-after.json) hashes |

The final three authored source hashes are retained in [authored-source-sha256.json](../../../../.context/metal-documentation-final-20261109/authored-source-sha256.json). The earlier typecheck and link-check logs also remain in that local directory.

## Prior validation and known baseline limit

The [original independent documentation review](review-documentation.md) records the first content version's reported Next/OpenNext builds, one-index gate and focused local Worker checks of routes, navigation, blog listing, search and exports. The reviewer independently inspected the generated 30-page, seven-blog one-index result and page HTML. The original author retained no standalone command logs for those checks. Those observations retain their original source and evidence limits. This final scoped pass does not relabel them as fresh rendered-HTTP or Worker checks.

That review also records the existing `test:blog`, `test:mcp` and `test:mcp:analytics` failure: query `deterministic simulation` ranks workload identity above deterministic-simulation testing. The unchanged archived website baseline reproduced it. Later analytics assertions remain unexecuted. No unrelated ranking change or rerun is part of this reconciliation, and no record claims all website acceptance suites passed.

## Review and finalization boundary

The original documentation approval covers the earlier pending-verification prose. The [final independent slice review](review-slice.md) is APPROVED with no open feature finding. This reconciled content requires its own independent documentation review, which remains pending when this record is written. This author does not approve or finalize the feature.

No production code, test body, model, native evidence, review artifact, DES phase event or unrelated dirty file was changed. No native, mutation, model, deployment, publication, push or cleanup command was run. The commit is limited to the two pages, the non-normative DELIVER disposition and this record.
