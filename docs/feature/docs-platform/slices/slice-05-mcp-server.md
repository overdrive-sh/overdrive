# Slice 05 — MCP Server (search_docs + get_doc)

The bespoke docs-MCP server: a Next route handler at `/mcp` (Streamable HTTP,
Node runtime, stateless) exposing two tools over the SAME build-time index.

## Learning hypothesis

> We believe a stateless MCP endpoint at `/mcp` exposing `search_docs(query)`
> and `get_doc(url)→getLLMText` lets a real MCP client ground its answers in
> current Overdrive docs. We will know this is true when Maya configures the
> endpoint once and her agent calls `search_docs` then `get_doc` and produces
> a job spec that validates first try.

This is the strategic differentiator's foundation — the agent-facing tool
surface that makes Overdrive a first-class context for coding agents.

## User-visible value

Maya's agent **autonomously grounds itself in real Overdrive docs** via MCP
tools, instead of hallucinating. The output runs. Maps to J-DOCS-002.

## Story

- **US-05** — Agent queries docs via the MCP endpoint and grounds its answer (J-DOCS-002)

## IN scope

- `app/mcp/route.ts` — Streamable HTTP, Node runtime, stateless (no Durable
  Object). Stateless fits a docs-query server.
- `search_docs(query)` → run the in-memory Orama query (the slice-03 index,
  fourth consumer of ONE index), return ranked `{ title, url, excerpt }`.
- `get_doc(url)` → look up the page, return `getLLMText(page)` (the slice-04
  primitive). Honest not-found when the URL does not resolve — never fabricate.
- A docs page documenting how to configure the endpoint (Maya copies it once).
- zod input schemas; `{ content: [{ type: 'text', text }] }` returns.

## OUT of scope

- Tool-call analytics logging (slice 06 — added as a wrapper around these
  handlers, deliberately deferred so the tools work first).
- OAuth / auth on the endpoint — public docs; auth is a possible later concern.
- `McpAgent` / per-session state — stateless only.
- Blog content in MCP results (joins when slice 07 lands).

## Acceptance (verifies elevator pitch After→sees)

- An MCP client (e.g. MCP Inspector) connects to `/mcp` and lists the two tools.
- `search_docs('submit a job')` returns ranked results whose top hit is the
  relevant how-to page URL.
- `get_doc(<that url>)` returns clean markdown (identical content to the
  slice-04 `.md` export for the same page).
- `get_doc(<nonexistent url>)` returns an honest not-found, not a fabricated page.

## Effort

≤ 1 day.

## Dependencies

- Slice 03 (the Orama index `search_docs` reuses) and slice 04 (the
  `getLLMText` primitive `get_doc` reuses).
