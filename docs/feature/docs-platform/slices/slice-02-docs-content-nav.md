# Slice 02 — Real Docs Content + Nav

Layer real Overdrive documentation onto the skeleton: multiple MDX pages
organised in a navigation tree, full `DocsLayout` with sidebar + TOC.

## Learning hypothesis

> We believe real Overdrive docs authored as MDX, organised via `meta.json`,
> render correctly with working navigation and TOC. We will know this is true
> when Priya browses from a concept page to a how-to page via a sidebar link
> and an in-page link, with no dead ends.

Validates the authoring workflow (MDX + meta) and the browse experience —
the foundation for J-DOCS-001's "browse to an answer" path.

## User-visible value

Priya can **browse a real, multi-page docs tree** and follow links between
pages without dead ends. Real answers exist to be found. Maps to J-DOCS-001.

## Story

- **US-02** — Browse real Overdrive docs with working navigation (J-DOCS-001)

## IN scope

- A starter set of real docs pages (concepts + at least one how-to) authored
  as MDX in `content/docs/`. Content seeded from the whitepaper/existing docs;
  must describe ONLY implemented behaviour (no aspirational docs).
- `meta.json` navigation ordering; `DocsLayout` sidebar + TOC.
- Cross-page links (concept → how-to) that resolve.
- Fumadocs MDX components (tabs/accordions/steps) available to authors.

## OUT of scope

- Search (slice 03) — browsing only here.
- llms export, MCP, blog, landing.
- API playground (`fumadocs-openapi`) — possible later; not this slice.
- Exhaustive docs coverage — a credible starter set, not the whole corpus.

## Acceptance (verifies elevator pitch After→sees)

- `GET /docs/<concept-page>` renders the concept with sidebar + TOC.
- A sidebar link and an in-page link both navigate to the linked how-to page.
- No page in the starter set is a stub or describes unimplemented behaviour.

## Effort

≤ 1 day.

## Dependencies

- Slice 01 (skeleton + `baseOptions()` shell + `lib/source.ts`).
