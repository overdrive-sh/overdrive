-- Migration 0001 — MCP tool-call analytics log (ADR-0056, docs-platform slice 06).
--
-- One row per MCP tool invocation, written best-effort (C-7): the write is
-- fire-and-forget via ctx.waitUntil() + catch-swallow in app/mcp/route.ts and
-- MUST NEVER block, delay, or alter the tool response. The maintainer reads
-- trends (KPI-4 volume, KPI-5 zero-result coverage gaps), not an audit ledger,
-- so lossy-under-failure is by design.
--
-- Schema is the SSOT block `d1_tool_calls_schema` in docs/product/kpi-contracts.yaml.
--
-- Apply locally (dev, against the local miniflare D1):
--   bunx wrangler d1 migrations apply ANALYTICS_DB --local
-- Apply remotely (DEVOPS wave, once the D1 database is provisioned):
--   bunx wrangler d1 migrations apply ANALYTICS_DB --remote

CREATE TABLE IF NOT EXISTS tool_calls (
	id           INTEGER PRIMARY KEY AUTOINCREMENT,
	tool         TEXT    NOT NULL, -- 'search_docs' | 'get_doc'
	query        TEXT    NOT NULL, -- the search_docs query string, or the get_doc url
	ts           INTEGER NOT NULL, -- unix epoch ms of the tool call
	result_count INTEGER NOT NULL  -- count of results returned; 0 marks a coverage gap (KPI-5)
);

-- Supports the maintainer's headline query (KPI-5 / J-DOCS-003):
--   SELECT query, COUNT(*) FROM tool_calls
--   WHERE tool = 'search_docs' AND result_count = 0
--   GROUP BY query ORDER BY 2 DESC;
-- The (tool, result_count, query) shape lets the zero-result GROUP BY be served
-- from the index without a full table scan.
CREATE INDEX IF NOT EXISTS idx_tool_calls_zero_result
	ON tool_calls (tool, result_count, query);
