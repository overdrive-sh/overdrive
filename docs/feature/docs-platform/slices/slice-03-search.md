# Slice 03 — Search

Add Orama search over the build-time docs index: the `/api/search` route
handler and the Cmd+K search dialog.

## Learning hypothesis

> We believe an in-Worker Orama index built from `source.getPages()` returns
> relevant results for technical queries fast enough to feel instant. We will
> know this is true when Priya types a technical term, gets a ranked result
> against the real corpus, and clicks through to the right page.

This is the first appearance of the strategic ONE-INDEX invariant: the index
built here is the same one MCP (slice 05) and llms export (slice 04) consume.

## User-visible value

Priya can **search and find an answer in seconds** instead of browsing.
Time-to-first-answer collapses. Maps to J-DOCS-001 (the headline outcome).

## Story

- **US-03** — Find a docs answer fast via search (J-DOCS-001)

## IN scope

- `app/api/search/route.ts` with `export const { GET } = createFromSource(source)`
  (the documented Next default).
- The default Fumadocs fetch-based `SearchDialog` via `RootProvider`
  (Cmd+K hotkey), pointed at `/api/search`.
- In-Worker Orama index from the SAME build-time `source` as slice 02
  (one index, first consumer).
- Node runtime (never edge). The 128 MB isolate ceiling bounds the index —
  fine for the current corpus; revisit only if the corpus grows large.

## OUT of scope

- External search (Orama Cloud / Algolia) — only if the corpus outgrows the
  in-Worker ceiling later; not now.
- MCP search_docs (slice 05) — reuses this index but is a separate consumer.
- Search analytics — the MCP analytics loop (slice 06) is the telemetry
  story; browser-search click analytics is a possible later addition.

## Acceptance (verifies elevator pitch After→sees)

- Cmd+K opens the dialog; typing a known technical term returns ≥1 ranked
  result whose top hit is the relevant concept/how-to page.
- Clicking a result navigates to that page.
- A query for an answerable term does not return empty.

## Effort

≤ 1 day.

## Dependencies

- Slice 02 (real content to index + the `source` loader).
