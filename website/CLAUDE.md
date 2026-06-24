# website/ — overdrive.sh docs platform

This subtree is the **overdrive.sh documentation / marketing / agent-discovery
site**: Fumadocs (v16) on Next.js (App Router, RSC) + React 19, deployed to
Cloudflare Workers via `@opennextjs/cloudflare` (OpenNext). It is
**architecturally independent of the Rust workspace** and **EXEMPT from every
Rust rule** in the repo (`.claude/rules/*`, the root `CLAUDE.md` conventions):
no cargo / nextest / Lima / dst-lint / cargo-mutants here. Toolchain is **bun**
+ `tsc` + `next build` + `wrangler` + `@opennextjs/cloudflare`. Gates: `bun run
typecheck`, `bun run lint`, `bun run build`, `bunx opennextjs-cloudflare build`,
and the glue checks `bun run {assert:one-index,test:mcp,test:mcp:analytics,test:blog}`.
Full record: `docs/feature/docs-platform/feature-delta.md`.

## Content authoring — the technical-writer skill is MANDATORY

**All written content for this site MUST be authored through the
`technical-writer` skill. Activate it before writing or editing ANY content,
and use it for the whole content pass — no exceptions.**

"Content" means every prose/authored surface, specifically:

- **Docs** — every `.mdx` under `content/docs/` (concepts, how-tos, reference,
  explanations).
- **Blog** — every post under `content/blog/`.
- **Landing / marketing copy** — the hero, value props, and section text in
  `app/(home)/page.tsx` and any future marketing pages.
- Any user-facing prose added to a component (empty states, callouts, nav
  labels with sentences, meta descriptions).

The rule applies to **new content AND edits to existing content**. Do not
hand-write or hand-edit site content outside the technical-writer skill, even
for "small" changes — activate the skill and let it drive the writing.

This does **not** apply to code, config, types, route handlers, build scripts,
or test scripts — only to authored content/prose. For those, the normal
engineering flow applies.

### Why

The site is the product's public voice and its agent-grounding corpus (the same
content feeds the docs UI, search, the MCP `get_doc`/`search_docs` tools, and
the `llms.txt` export — one index, many consumers). Consistent, high-quality,
on-voice technical writing is load-bearing: agents ground on it, evaluators
judge the product by it. The technical-writer skill enforces that bar uniformly
instead of leaving voice/structure to whoever happens to be editing.

### Content constraints that still hold (C-6)

Document **only real, implemented behaviour** — never describe features as
shipped that do not exist. Source from the whitepaper / `docs/product/` /
verified runtime behaviour. The technical-writer skill produces the writing;
this accuracy constraint is non-negotiable on top of it.

### Current state only — describe what is, not what was (C-7)

Write every page as a description of the system **as it is now**. This is a
greenfield project with no users, so a feature's previous state is not content
anyone needs. Do **not** narrate evolution: no "was staged, now ships," no
"previously X, now Y," no "this used to require Z," and no "now" / "today"
markers whose only job is to contrast with a past. When behaviour changes,
rewrite the affected prose to the new current state and delete the description
of the old one — do not layer a "what changed" note on top of it. Genuine
current boundaries — what the system does **not** do yet — are current state and
belong in plain present tense ("the ingress gateway is unbuilt"), not framed as a
transition. This mirrors the Rust workspace's single-cut greenfield discipline:
removed is removed, and the docs track head state, not its history.
