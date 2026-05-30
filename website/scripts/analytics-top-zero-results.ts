/**
 * Maintainer read path for the MCP tool-call analytics log (US-06 / J-DOCS-003,
 * ADR-0056, KPI-5). Diego runs this to see what agents actually ask:
 *
 *   - TOP ZERO-RESULT search_docs queries — the coverage-gap signal (KPI-5):
 *       which topic did an agent search for and get NOTHING? That is the next
 *       docs page to write, ordered by real demand.
 *   - TOP search_docs queries overall — what is asked most (KPI-4 demand shape).
 *
 * These are the two `maintainer_queries` from kpi-contracts.yaml's
 * `d1_tool_calls_schema`, run directly against D1 — the whole point of choosing
 * D1 over Analytics Engine (ADR-0056) is that this is one line of ordinary SQL.
 *
 * Usage:
 *   bun run analytics:top-zero-results            # local dev D1 (miniflare)
 *   bun run analytics:top-zero-results --remote   # provisioned D1 (DEVOPS wave)
 *
 * No dashboard UI — a query + a printed table is enough to validate the slice's
 * learning hypothesis (the maintainer can name top asked + top zero-result
 * topics). Equivalent raw invocation, if you prefer wrangler directly:
 *   bunx wrangler d1 execute ANALYTICS_DB --local --command \
 *     "SELECT query, COUNT(*) AS n FROM tool_calls \
 *      WHERE tool='search_docs' AND result_count=0 GROUP BY query ORDER BY n DESC LIMIT 25;"
 */
import { spawnSync } from "node:child_process";

const BINDING = "ANALYTICS_DB";

const TOP_ZERO_RESULT = `
SELECT query, COUNT(*) AS n FROM tool_calls
WHERE tool = 'search_docs' AND result_count = 0
GROUP BY query ORDER BY n DESC LIMIT 25;`.trim();

const TOP_QUERIES = `
SELECT query, COUNT(*) AS n FROM tool_calls
WHERE tool = 'search_docs'
GROUP BY query ORDER BY n DESC LIMIT 25;`.trim();

function runQuery(label: string, sql: string, remote: boolean): void {
	process.stdout.write(`\n── ${label} ──\n`);
	const scope = remote ? "--remote" : "--local";
	const result = spawnSync(
		"bunx",
		["wrangler", "d1", "execute", BINDING, scope, "--command", sql],
		{ stdio: "inherit", env: process.env },
	);
	if (result.status !== 0) {
		process.stderr.write(
			`\n✗ query "${label}" failed (exit ${result.status ?? "signal"}). ` +
				`Has the migration been applied? Run:\n` +
				`    bunx wrangler d1 migrations apply ${BINDING} ${scope}\n`,
		);
		process.exit(result.status ?? 1);
	}
}

function main(): void {
	const remote = process.argv.includes("--remote");
	process.stdout.write(
		`MCP tool-call analytics — maintainer view (${remote ? "remote" : "local"} D1)\n`,
	);
	runQuery("Top ZERO-RESULT queries (coverage gaps — KPI-5)", TOP_ZERO_RESULT, remote);
	runQuery("Top queries overall (demand shape — KPI-4)", TOP_QUERIES, remote);
}

main();
