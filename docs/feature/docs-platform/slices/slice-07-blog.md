# Slice 07 — Blog

A blog at `/blog` as a second content collection feeding the same index.
Fumadocs has no turnkey blog layout — BYO list + post pages.

## Learning hypothesis

> We believe a second `defineCollections({ type: 'doc' })` collection for blog
> posts, with hand-rolled list/post pages, renders correctly and feeds the
> same build-time index as docs. We will know this is true when a visitor
> reads a blog post AND that post is searchable + MCP-discoverable for free.

## User-visible value

Priya (and any visitor) can **read Overdrive blog posts**; the posts are
automatically searchable and agent-discoverable because they join the one
index. Maps primarily to J-DOCS-001, with spillover to J-DOCS-002 (agents can
ground on blog content too).

## Story

- **US-07** — Read blog posts that are also searchable + MCP-discoverable (J-DOCS-001)

## IN scope

- A second content collection (`content/blog/`, flat posts, no catch-all).
- Hand-rolled list page (`app/(home)/blog/page.tsx`) and post page
  (`app/(home)/blog/[slug]/page.tsx`) — the documented Next components.
- Frontmatter schema (title, date, author/summary) for posts.
- The blog collection joins the SAME build-time index → searchable (slice 03),
  exported (slice 04), MCP-discoverable (slice 05) with no extra wiring.
- The shared `baseOptions()` nav shell (blog under the same shell as / and /docs).

## OUT of scope

- RSS / OG image generation — desirable but not required to validate the slice
  (possible follow-up; surfaced as a deferral candidate in feature-delta.md).
- Comments, tags/categories taxonomy beyond basic frontmatter.
- A bespoke blog visual redesign — use the documented components.

## Acceptance (verifies elevator pitch After→sees)

- `GET /blog` lists posts (title, date) under the shared nav shell.
- `GET /blog/<slug>` renders the post body.
- A term unique to a blog post returns that post in search (slice 03) AND in
  MCP `search_docs` (slice 05) — proving it joined the one index.

## Effort

≤ 1 day.

## Dependencies

- Slice 01 (shell + source). Indexing benefits land once 03/04/05 exist;
  blog content joins those consumers automatically.
