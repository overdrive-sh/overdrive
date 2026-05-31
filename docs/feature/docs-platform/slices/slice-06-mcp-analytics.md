# Slice 06 — MCP Tool-Call Analytics Loop

Wrap the MCP tool handlers with a best-effort logging call that records
`{tool, query, ts, result_count}` to a Cloudflare binding — the strategic
docs-prioritisation signal.

## Learning hypothesis

> We believe agent tool-call telemetry (what agents search for, whether they
> found results, and the search→get_doc→re-search trajectory) is a BETTER
> docs-prioritisation signal than a chat widget, because the query is
> task-contextual. We will know this is true when the maintainer reads the
> aggregated log and can name the top asked-but-unanswered topics.

This is the feature's central thesis. The slice validates that the signal is
capturable and legible — the analysis that proves/disproves the thesis is the
maintainer reading it (J-DOCS-003).

## User-visible value

The docs maintainer (Diego, J-DOCS-003) can **see what agents actually ask**
and prioritise docs by real demand. Maps to J-DOCS-003. (The agent and Maya
are unaffected — logging is invisible to them and must never break a response.)

## Story

- **US-06** — Maintainer sees agent demand from logged MCP tool calls (J-DOCS-003)

## IN scope

- A logging wrapper around the slice-05 `search_docs` / `get_doc` handlers
  writing `{tool, query, ts, result_count}` to a Cloudflare binding.
- **Binding choice is a DESIGN-wave decision** — Analytics Engine vs D1. This
  slice specifies WHAT is logged and the best-effort contract; DESIGN picks WHICH.
- Best-effort contract: a logging failure MUST NOT block or fail the tool
  response. The answer always returns; the log is fire-and-forget.
- A way for the maintainer to read the aggregate (a query or simple view) —
  enough to name top queries and zero-result queries.

## OUT of scope

- A full analytics dashboard / UI — a query or minimal view suffices to
  validate the thesis; a polished dashboard is a later concern.
- Browser-search click analytics — this slice is MCP tool calls only.
- PII handling beyond not logging anything more than the four fields (queries
  are task text, not user identity).

## Acceptance (verifies elevator pitch After→sees)

- After an agent makes `search_docs` / `get_doc` calls, the maintainer reads
  the binding and sees rows with `{tool, query, ts, result_count}`.
- A `search_docs` call with zero results produces a row with `result_count = 0`
  (the zero-result signal is captured — this is the coverage-gap signal).
- Forcing a logging failure does NOT change the tool response the agent receives.

## Effort

≤ 1 day.

## Dependencies

- Slice 05 (the MCP handlers being wrapped).
