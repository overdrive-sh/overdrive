# Documentation and blog review — public ingress gateway

- Content: `website/content/blog/public-ingress-gateway.mdx`, `website/content/docs/concepts/public-ingress-gateway.mdx`, `website/content/docs/how-to/expose-a-service-publicly.mdx`, and the gateway edits to `concepts/{architecture,identity,workload-identity-lifecycle}.mdx`, `how-to/{deploy-a-workload,run-verification-on-metal}.mdx`, `reference/cli.mdx` and the `meta.json` files
- Bar: `.claude/skills/technical-writer/SKILL.md`, `.claude/rules/finalize.md`, `website/CLAUDE.md`, CLAUDE.md "Scope limitations are deferrals too"
- Reviewer: independent review agents (Sonnet), dispatched with the rules, the content and the sources of truth, never the author's summary
- Date: 2026-11-13
- Final verdict: **APPROVED** at `116aa6ba2`

## Rounds

1. **CHANGES_REQUESTED** (four blocking): the one-Route limit and the fixed runtime limits had no issue citation (#352, #353), the blog repeated the uncited one-Route limit, and the blog carried internal design IDs.
2. **CHANGES_REQUESTED** against the extended technical-writer skill (thirteen findings): evidence scoped to one recorded run and not to streaming; the credential bug attributed to the wrong run; untracked limitations ("VM workloads only" was a platform fact and was removed; CLI verbs are tracked in #355, unlinked by the owner's ruling); non-descriptive issue links; terms used before definition; banned words; process wording; the hyper decision story; how-to steps that were not one action each.
3. **CHANGES_REQUESTED** (three blocking): the how-to checked gateway status before a Route existed, omitted the gateway's key-encryption key, and the limits table left out response-side limits. Non-blocking: the hook cost is a 30-run median, the rejected fold rests on verifier instruction counts and the 5% growth rule, `healthy` is ignored because of an unexplained flap, the memory figures are a fit plus an asserted bound, `manual` as the status endpoint prints it, a Route defined at first use, the generation is a content hash, path-match wording, "account" not "user", the architecture page's gateway contrast, and the unsupported protocol upgrade.
4. **APPROVED**: every round-3 finding resolved, each checked against the code, ADRs, spike findings or `examples/public-ingress-gateway/run-example.sh`; no new finding blocking. One non-blocking note: a long source line in the concept page that does not affect the rendered text.

## Validation

- `bun run build`: passes.
- `bun run test:blog`: could not run. It starts `wrangler dev`, which needs `bunx`, and the local proto install of bun 1.3.5 has no `bunx` binary. Environmental; the test never started a server.
