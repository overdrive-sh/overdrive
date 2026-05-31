# ADR-0056 — docs-platform: D1 analytics binding + best-effort tool-call logging contract

## Status

Accepted. 2026-05-30. Decision-makers: Morgan (proposing); user (GUIDE-mode
decision D-B). Tags: docs-platform, website, overdrive.sh, analytics, mcp,
best-effort, application-arch.

**Scope note**: overdrive.sh website (`website/` subtree); architecturally
independent of the Rust platform. Resolves DISCUSS Deferral D-2 ("Analytics
binding choice — Analytics Engine vs D1, a DESIGN decision").

References DISCUSS US-06 (J-DOCS-003), constraint C-7 (analytics is
best-effort), the journey's `tool_call_log_record` shared artifact, and
research Finding 3.4.3 (Workers Analytics Engine / D1 as the edge-native sinks).

## Context

US-06 (J-DOCS-003) requires the docs maintainer (Diego) to read what agents
actually ask: rows of `{ tool, query, ts, result_count }` from MCP tool calls,
with a first-class need to find **top zero-result queries** ("which topic did an
agent search for and get nothing?") so the next docs page is written against
real demand. A zero-result `search_docs` must produce a row with
`result_count = 0`.

Two hard requirements frame the decision:

1. **The maintainer's headline query is an aggregation over the log.** "Top
   zero-result queries this month" is `SELECT query, COUNT(*) FROM tool_calls
   WHERE result_count = 0 AND ts >= … GROUP BY query ORDER BY COUNT(*) DESC`.
2. **Logging is best-effort (C-7).** A failure of the logging path must NEVER
   alter or delay the MCP tool response. US-06 AC: "Forcing a logging failure
   does not change the tool response."

Research Finding 3.4.3 names two edge-native sinks: **Workers Analytics Engine**
and **D1**. Both are bindings declared in `wrangler.jsonc` and both are
queryable by the maintainer.

## Decision

**The analytics binding is D1** (Cloudflare's serverless SQLite). The MCP route
handler (ADR-0055) writes one row per tool call to a `tool_calls` table:
`{ tool TEXT, query TEXT, ts INTEGER, result_count INTEGER }`.

**Rationale for D1 over Analytics Engine**: the maintainer's primary job is an
ad-hoc SQL aggregation with a `WHERE result_count = 0 … GROUP BY query` shape.
On D1 that is one line of real SQL the maintainer runs directly
(`wrangler d1 execute` or the dashboard console). Analytics Engine is optimised
for high-cardinality time-series writes queried via its SQL API over HTTP with a
sampling model; the zero-result-query aggregation is awkward there and the
sampling could under-count the exact low-frequency long-tail queries that ARE
the coverage-gap signal. At docs-MCP write volume (human-paced agent traffic,
not request-per-packet telemetry) D1's write throughput is a non-issue.

**The best-effort logging contract (HARD REQUIREMENT, C-7)** — the logging
wrapper around each tool body MUST satisfy all three:

1. **Fire-and-forget via `ctx.waitUntil()`.** The D1 write is handed to
   `ctx.waitUntil(logToolCall(...))` so it runs after the response is returned;
   it never sits on the response's critical path. The tool response is computed
   and returned regardless of whether the write has started, completed, or
   failed.
2. **Catch-and-swallow.** `logToolCall` wraps the D1 `INSERT` in a
   `try/catch` that swallows every error (binding unavailable, D1 throttle,
   malformed row). A logging failure is, at most, a dropped row — never a
   thrown error that propagates into the tool handler.
3. **No `await` on the log before responding.** The tool body computes its
   result (`searchIndex(query)` or `getLLMText(page)`), schedules the log via
   `ctx.waitUntil`, and returns. The result is never a function of the log
   outcome.

This contract is the single most important behavioural property of the slice
and is independently testable: a forced logging failure (e.g. an unbound /
deliberately-erroring D1 stub) MUST leave the tool response byte-identical to
the success case (US-06 AC).

## Alternatives considered

### Alternative A — Workers Analytics Engine (REJECTED — the pass-1 alternative)

- **Why considered**: purpose-built for write-heavy event telemetry; the other
  research-named sink (Finding 3.4.3); zero schema management.
- **Why rejected**: the maintainer's defining query is a grouped zero-result
  aggregation over the exact (non-sampled) long-tail. Analytics Engine's
  sampling model and HTTP SQL-API ergonomics make that the hard path, where on
  D1 it is one line of ordinary SQL. The write-volume advantage Analytics
  Engine offers is irrelevant at human-paced agent traffic. Choosing it would
  optimise a dimension (write throughput) the feature does not stress while
  degrading the dimension (ad-hoc low-cardinality aggregation) it depends on.

### Alternative B — Log synchronously, then respond (REJECTED — violates C-7)

Write the row with `await` before returning the tool result.

- **Why rejected**: a slow or failed D1 write would delay or break the agent's
  answer — the exact failure mode C-7 and the US-06 AC forbid. The analytics
  loop is a maintainer convenience; it must never be a dependency of the answer
  path.

### Alternative C — KV instead of D1 (REJECTED)

- **Why rejected**: KV is a key-value store with no `GROUP BY`; the maintainer
  would have to export and aggregate client-side. D1 gives the aggregation for
  free, which is the entire point of US-06.

## Consequences

**Positive**

- The maintainer's headline question ("top zero-result queries") is a one-line
  `SELECT` — US-06's value proposition lands directly.
- Real relational schema means future maintainer queries (frequency over time,
  per-tool breakdown, follow-up trajectory) are also plain SQL.
- The `ctx.waitUntil` + catch-swallow contract makes the C-7 guardrail
  structural and testable, not aspirational.

**Negative / accepted trade-offs**

- A best-effort log drops rows silently under D1 unavailability. Accepted by
  design: an occasionally-incomplete prioritisation signal is strictly better
  than a logging path that can break an agent's answer. The maintainer reads
  trends, not an audit ledger; lossy-under-failure is the correct posture.
- D1 schema is a migration surface the website now owns (one `CREATE TABLE`).
  Trivial at this scale; declared in the `website/` deploy config.
- The `tool_calls` table is the binding name in `wrangler.jsonc`; the actual D1
  database provisioning + binding is DEVOPS-wave (same wave as the custom-domain
  wiring, D-F). The skeleton may run against a local/dev D1 or with logging
  no-op'd — and because the contract is best-effort, an unbound D1 in the
  skeleton degrades to "no rows," not "broken tools."
