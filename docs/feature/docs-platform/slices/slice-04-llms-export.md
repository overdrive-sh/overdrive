# Slice 04 — LLM Content Export

Expose the docs corpus as machine-readable text: `llms.txt`, `llms-full.txt`,
and per-page `.md`, all via the `getLLMText` primitive.

## Learning hypothesis

> We believe the `getLLMText(page)` primitive over the build-time source
> produces clean, agent-consumable markdown for the whole corpus. We will know
> this is true when an agent (or a curl) fetches `/llms-full.txt` or a
> per-page `.md` and gets clean title + processed markdown, NOT HTML chrome.

This proves the content-extraction primitive that MCP `get_doc` (slice 05)
depends on — de-risking the MCP slice by validating the primitive first.

## User-visible value

Maya's agent (or any llms.txt-consuming tool) can **pull the corpus as clean
markdown** today, even before the bespoke MCP server exists. A zero-config
agent path (point `mcpdoc` at `/llms.txt`) works immediately. Maps to J-DOCS-002.

## Story

- **US-04** — Export docs as clean LLM-readable text (J-DOCS-002)

## IN scope

- `llms.txt` (index via `llms(source).index()`).
- `llms-full.txt` (all pages mapped through `getLLMText()` and joined).
- Per-page `.md` access (append `.md` to a doc URL).
- `includeProcessedMarkdown: true` in source config (required by `getLLMText`).
- Accept-header negotiation via `isMarkdownPreferred()` where applicable.
- All on the SAME build-time source as search (one index, second/third consumer).

## OUT of scope

- The bespoke MCP server (slice 05) — this slice provides the content surface
  MCP reuses, not the MCP tools themselves.
- Tool-call analytics (slice 06).
- Blog content in the export (blog lands in slice 07; it joins the export then).

## Acceptance (verifies elevator pitch After→sees)

- `GET /llms.txt` returns an index of doc URLs.
- `GET /llms-full.txt` returns clean concatenated markdown (no HTML chrome).
- `GET /docs/<page>.md` returns that page's clean title + processed markdown.

## Effort

≤ 1 day.

## Dependencies

- Slice 02 (content + source). Independent of slice 03 (search), but both
  consume the same index.
