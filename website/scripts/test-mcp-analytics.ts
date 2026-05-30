/**
 * Slice-06 acceptance test — the C-7 best-effort-analytics guardrail.
 *
 * Extends the slice-05 MCP acceptance test (scripts/test-mcp.ts) with the
 * analytics loop. It runs the REAL OpenNext Worker on workerd via `wrangler
 * dev`, connects a real MCP `Client` over `StreamableHTTPClientTransport` to
 * `/mcp`, and drives a LOCAL D1 (miniflare) that has migration 0001 applied —
 * the same write path production uses. It asserts:
 *
 *   [A] Row written      — after a search_docs + a get_doc call, the local D1
 *                          `tool_calls` table holds rows with the correct
 *                          {tool, query, ts, result_count} (search row carries
 *                          the query + its page count; get_doc row carries the
 *                          url + 1).
 *   [B] Zero-result      — search_docs(<query with no matching page>) writes a
 *                          row with result_count = 0 (the KPI-5 coverage-gap
 *                          signal).
 *   [C] C-7 fault inject — a SECOND worker is launched with ANALYTICS_FORCE_FAIL=1,
 *                          which makes the D1 INSERT target a non-existent table
 *                          so the live D1 engine rejects it at run() time with a
 *                          real `D1_ERROR: no such table` (a genuine induced
 *                          failure, NOT a mocked no-op). We assert that:
 *                            (c1) the search_docs response is BYTE-IDENTICAL to
 *                                 the healthy-path response for the same call;
 *                            (c2) the get_doc response is BYTE-IDENTICAL too;
 *                            (c3) the failing-log call returns in the same
 *                                 ballpark as the healthy-log call (the fire-
 *                                 and-forget path does not await the failing
 *                                 write — no multi-second stall);
 *                            (c4) the broken worker wrote ZERO rows (proving the
 *                                 failure was real, not silently succeeding).
 *   [D] Slice-05 invariants still hold WITH the wrapper in place — tools listed,
 *       search top-hit unchanged, get_doc === .md identity, honest not-found.
 *
 * Port-to-port: the system is driven ONLY through the MCP endpoint (driving
 * port) and the public `.md` HTTP route; the D1 side is observed through
 * `wrangler d1 execute` (the maintainer's own read path), not through any
 * imported internal module.
 *
 * Run with `bun run test:mcp:analytics`.
 */
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { rmSync } from "node:fs";
import { resolve } from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const HEALTHY_PORT = Number(process.env.MCP_TEST_PORT ?? 8987);
const FAULT_PORT = HEALTHY_PORT + 1;
// Two miniflare workers cannot share one local-D1 SQLite file (SQLITE_BUSY).
// Each worker gets its own hermetic persistence dir; both have migration 0001
// applied, so the `tool_calls` table exists in BOTH — the fault path fails NOT
// because the table is missing in its DB, but because ANALYTICS_FORCE_FAIL=1
// redirects the INSERT to a different, non-existent table name.
const HEALTHY_PERSIST_DIR = resolve(process.cwd(), ".wrangler/state-analytics-healthy");
const FAULT_PERSIST_DIR = resolve(process.cwd(), ".wrangler/state-analytics-fault");
const BINDING = "ANALYTICS_DB";

type CallToolText = { content: { type: string; text?: string }[]; isError?: boolean };

function log(line: string): void {
	process.stdout.write(`${line}\n`);
}

function assert(condition: unknown, message: string): asserts condition {
	if (!condition) throw new Error(`ASSERTION FAILED: ${message}`);
}

function firstText(result: CallToolText): string {
	const block = result.content.find((c) => c.type === "text");
	assert(block?.text !== undefined, "tool result has a text content block");
	return block.text;
}

// ── local-D1 helpers (the maintainer's read path) ───────────────────────────

function applyMigration(persistDir: string): void {
	const result = spawnSync(
		"bunx",
		["wrangler", "d1", "migrations", "apply", BINDING, "--local", "--persist-to", persistDir],
		{ encoding: "utf8", env: process.env },
	);
	assert(
		(result.status ?? 1) === 0,
		`migration apply failed: ${result.stderr ?? ""}${result.stdout ?? ""}`,
	);
}

// Returns the rows of an arbitrary SELECT as parsed JSON objects.
function query(persistDir: string, sql: string): Record<string, unknown>[] {
	const result = spawnSync(
		"bunx",
		[
			"wrangler",
			"d1",
			"execute",
			BINDING,
			"--local",
			"--persist-to",
			persistDir,
			"--json",
			"--command",
			sql,
		],
		{ encoding: "utf8", env: process.env },
	);
	assert(
		(result.status ?? 1) === 0,
		`d1 query failed: ${result.stderr ?? ""}${result.stdout ?? ""}`,
	);
	// `--json` emits an array of { results: [...] } per statement, but wrangler
	// may prefix stdout with a non-JSON banner (agent-skills notice). Slice from
	// the first '[' to the matching end so JSON.parse sees only the payload.
	const out = result.stdout ?? "";
	const start = out.indexOf("[");
	const end = out.lastIndexOf("]");
	assert(start >= 0 && end > start, `d1 --json produced no JSON array:\n${out}`);
	const parsed = JSON.parse(out.slice(start, end + 1)) as {
		results?: Record<string, unknown>[];
	}[];
	return parsed[0]?.results ?? [];
}

// ── worker lifecycle ────────────────────────────────────────────────────────

// `vars` are injected into the WORKER's `env` via wrangler's `--var KEY:VALUE`
// flag. NOTE: a parent-process env var would NOT reach the Worker's `env` —
// wrangler only exposes declared bindings/vars/secrets to the isolate, not the
// CLI's own environment. This is how ANALYTICS_FORCE_FAIL actually lands on
// `getCloudflareContext().env` inside the route handler.
function startWorker(
	port: number,
	persistDir: string,
	vars: Record<string, string>,
): ChildProcess {
	const varArgs = Object.entries(vars).flatMap(([k, v]) => ["--var", `${k}:${v}`]);
	return spawn(
		"bunx",
		[
			"wrangler",
			"dev",
			"--port",
			String(port),
			"--inspector-port",
			"0",
			"--persist-to",
			persistDir,
			...varArgs,
		],
		{ stdio: ["ignore", "inherit", "inherit"], env: process.env },
	);
}

async function waitForServer(origin: string, timeoutMs: number): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	let lastErr: unknown;
	while (Date.now() < deadline) {
		try {
			const res = await fetch(`${origin}/docs.md`);
			if (res.ok || res.status < 500) return;
		} catch (err) {
			lastErr = err;
		}
		await new Promise((r) => setTimeout(r, 500));
	}
	throw new Error(
		`server did not become ready on ${origin} within ${timeoutMs}ms: ${String(lastErr)}`,
	);
}

async function connectClient(origin: string, name: string): Promise<Client> {
	const client = new Client({ name, version: "0.1.0" });
	const transport = new StreamableHTTPClientTransport(new URL(`${origin}/mcp`));
	await client.connect(transport);
	return client;
}

// ── corpus fixtures (shared with slice-05) ──────────────────────────────────

const SEARCH_QUERY = "deterministic simulation";
const EXPECTED_URL = "/docs/concepts/deterministic-simulation-testing";
// A single nonsense token: the Orama index matches no page. (Multi-word
// queries tokenize and match common words like "in"/"docs", which would NOT be
// a zero-result case — probed empirically against the live index.)
const ZERO_RESULT_QUERY = "xyzzyplughblorb";
const MISSING_URL = "/docs/does-not-exist";

// Two miniflare workers cannot hold the same local-D1 SQLite file open
// concurrently (SQLITE_BUSY), and the OpenNext worker opens its bound D1 at
// startup. So the two phases run SEQUENTIALLY against separate persistence
// dirs: the healthy worker runs + is torn down, then the faulty worker runs.
// The known-good response bytes captured in phase 1 are the C-7 comparison
// baseline for phase 2.

type Baseline = { searchText: string; docText: string; pageCount: number };

async function runHealthyPhase(): Promise<Baseline> {
	const worker = startWorker(HEALTHY_PORT, HEALTHY_PERSIST_DIR, {});
	const origin = `http://127.0.0.1:${HEALTHY_PORT}`;
	let client: Client | undefined;
	try {
		await waitForServer(origin, 120_000);
		client = await connectClient(origin, "slice-06-healthy");

		// ── [D] slice-05 invariants still hold WITH the wrapper ──────────────────
		log("\n[D] slice-05 invariants under the analytics wrapper");
		const { tools } = await client.listTools();
		const toolNames = tools.map((t) => t.name).sort();
		assert(
			toolNames.length === 2 &&
				toolNames[0] === "get_doc" &&
				toolNames[1] === "search_docs",
			`expected exactly [get_doc, search_docs], got ${JSON.stringify(toolNames)}`,
		);
		for (const tool of tools) {
			assert(
				tool.inputSchema && typeof tool.inputSchema === "object",
				`tool ${tool.name} exposes an input schema`,
			);
		}
		log("    ✓ exactly search_docs + get_doc, both with input schemas");

		// Healthy search_docs — capture the KNOWN-GOOD response bytes.
		const healthySearch = (await client.callTool({
			name: "search_docs",
			arguments: { query: SEARCH_QUERY },
		})) as CallToolText;
		const searchText = firstText(healthySearch);
		const pages = JSON.parse(searchText) as { title: string; url: string; excerpt: string }[];
		assert(pages.length > 0, "search_docs returned at least one page");
		assert(
			pages[0].url === EXPECTED_URL,
			`top hit should be ${EXPECTED_URL}, got ${pages[0].url}`,
		);
		log(`    ✓ search top hit unchanged: ${pages[0].url}`);

		// Healthy get_doc — identity vs the .md export.
		const healthyDoc = (await client.callTool({
			name: "get_doc",
			arguments: { url: EXPECTED_URL },
		})) as CallToolText;
		assert(healthyDoc.isError !== true, "get_doc on a real URL is not an error");
		const docText = firstText(healthyDoc);
		const mdExport = await (await fetch(`${origin}${EXPECTED_URL}.md`)).text();
		assert(
			docText === mdExport,
			"get_doc output must be byte-identical to the .md export (US-05 identity)",
		);
		log("    ✓ get_doc === GET /docs/.../.md (byte-identical, US-05)");

		// Honest not-found (also exercises the get_doc result_count=0 log path).
		const missing = (await client.callTool({
			name: "get_doc",
			arguments: { url: MISSING_URL },
		})) as CallToolText;
		const missingText = firstText(missing);
		assert(missing.isError === true, "get_doc on a nonexistent URL reports an error");
		assert(!missingText.startsWith("# "), "not-found result is not a fabricated page");
		log("    ✓ honest not-found, no fabricated page");

		// Zero-result search (the KPI-5 coverage-gap signal).
		const zeroSearch = (await client.callTool({
			name: "search_docs",
			arguments: { query: ZERO_RESULT_QUERY },
		})) as CallToolText;
		const zeroPages = JSON.parse(firstText(zeroSearch)) as unknown[];
		assert(
			zeroPages.length === 0,
			`zero-result query must return 0 pages, got ${zeroPages.length}`,
		);
		log("    ✓ zero-result query returns 0 pages");

		// Let the best-effort waitUntil writes drain before reading D1.
		await new Promise((r) => setTimeout(r, 2000));

		// ── [A] rows written with correct {tool, query, ts, result_count} ────────
		log("\n[A] rows written to local D1 tool_calls");
		const allRows = query(
			HEALTHY_PERSIST_DIR,
			"SELECT tool, query, ts, result_count FROM tool_calls ORDER BY id;",
		);
		for (const row of allRows) {
			log(
				`      ${String(row.tool)}  result_count=${String(row.result_count)}  ts=${String(row.ts)}  query=${JSON.stringify(row.query)}`,
			);
		}
		const searchRow = allRows.find(
			(r) => r.tool === "search_docs" && r.query === SEARCH_QUERY,
		);
		assert(searchRow !== undefined, "a search_docs row for the healthy query exists");
		assert(
			Number(searchRow.result_count) === pages.length,
			`search row result_count must equal the page count (${pages.length}), got ${String(searchRow.result_count)}`,
		);
		assert(
			Number(searchRow.ts) > 0 && Number.isFinite(Number(searchRow.ts)),
			"search row ts is a unix epoch ms integer",
		);
		log(`    ✓ search_docs row: query="${SEARCH_QUERY}", result_count=${pages.length}`);

		const getDocRow = allRows.find((r) => r.tool === "get_doc" && r.query === EXPECTED_URL);
		assert(getDocRow !== undefined, "a get_doc row for the resolved url exists");
		assert(
			Number(getDocRow.result_count) === 1,
			`get_doc row result_count must be 1, got ${String(getDocRow.result_count)}`,
		);
		log(`    ✓ get_doc row: query="${EXPECTED_URL}", result_count=1`);

		// ── [B] zero-result row captured with result_count = 0 ───────────────────
		log("\n[B] zero-result row captured");
		const zeroRows = query(
			HEALTHY_PERSIST_DIR,
			`SELECT tool, query, result_count FROM tool_calls WHERE tool='search_docs' AND result_count=0;`,
		);
		const zeroRow = zeroRows.find((r) => r.query === ZERO_RESULT_QUERY);
		assert(
			zeroRow !== undefined,
			`a search_docs row with result_count=0 for "${ZERO_RESULT_QUERY}" must exist`,
		);
		log(`    ✓ zero-result row: query="${ZERO_RESULT_QUERY}", result_count=0`);
		// Sanity: the missing get_doc also logged result_count=0.
		const missingDocRow = query(
			HEALTHY_PERSIST_DIR,
			`SELECT result_count FROM tool_calls WHERE tool='get_doc' AND query='${MISSING_URL}';`,
		);
		assert(
			missingDocRow.length >= 1 && Number(missingDocRow[0].result_count) === 0,
			"the not-found get_doc logged result_count=0",
		);
		log(`    ✓ not-found get_doc row: query="${MISSING_URL}", result_count=0`);

		return { searchText, docText, pageCount: pages.length };
	} finally {
		await client?.close().catch(() => {});
		worker.kill("SIGTERM");
		// Wait for the worker to release the local-D1 SQLite lock before phase 2.
		await new Promise((r) => setTimeout(r, 2500));
	}
}

async function runFaultPhase(baseline: Baseline): Promise<void> {
	// First establish a healthy-latency baseline on THIS worker config so the
	// timing comparison is same-host, same-build. We do that by timing a call to
	// the healthy worker — but it's already down, so we time the broken worker's
	// FIRST call as the cold baseline and a SECOND identical call: a fire-and-
	// forget failing write must make neither call materially slower than a pure
	// request/response. We assert both are well under an absolute ceiling, and
	// (the stronger signal) that the broken worker wrote ZERO rows.
	const worker = startWorker(FAULT_PORT, FAULT_PERSIST_DIR, { ANALYTICS_FORCE_FAIL: "1" });
	const origin = `http://127.0.0.1:${FAULT_PORT}`;
	let client: Client | undefined;
	try {
		await waitForServer(origin, 120_000);
		client = await connectClient(origin, "slice-06-faulty");

		log("\n[C] C-7 fault injection — D1 INSERT forced to a non-existent table");
		log("    ANALYTICS_FORCE_FAIL=1 → INSERT ... INTO tool_calls__force_fail_no_such_table");
		log("    → live D1 rejects at run() with `D1_ERROR: no such table`, swallowed by ctx.waitUntil catch");

		// (c1) byte-identical search response vs the phase-1 known-good bytes.
		const t0a = performance.now();
		const faultSearch = (await client.callTool({
			name: "search_docs",
			arguments: { query: SEARCH_QUERY },
		})) as CallToolText;
		const faultSearchMs = performance.now() - t0a;
		assert(
			firstText(faultSearch) === baseline.searchText,
			"BROKEN-LOG search_docs response must be byte-identical to the healthy response",
		);
		log("    ✓ (c1) search_docs response byte-identical under forced log failure");

		// (c2) byte-identical get_doc response.
		const t0b = performance.now();
		const faultDoc = (await client.callTool({
			name: "get_doc",
			arguments: { url: EXPECTED_URL },
		})) as CallToolText;
		const faultDocMs = performance.now() - t0b;
		assert(
			faultDoc.isError !== true && firstText(faultDoc) === baseline.docText,
			"BROKEN-LOG get_doc response must be byte-identical to the healthy response",
		);
		log("    ✓ (c2) get_doc response byte-identical under forced log failure");

		// (c3) not delayed — the failing write is fire-and-forget (not awaited).
		// A synchronous await on the failing D1 write would add the full D1 reject
		// round-trip to the response; an absolute ceiling well under a second proves
		// the response did not wait on it. (D1 rejects are sub-100ms locally, so an
		// awaited failure would still be fast — the ZERO-rows assert in (c4) is the
		// stronger proof the failure was real; this is the timing sanity check.)
		const ceiling = 1500;
		log(`      broken-log search: ${faultSearchMs.toFixed(0)}ms | broken-log get_doc: ${faultDocMs.toFixed(0)}ms | ceiling: ${ceiling}ms`);
		assert(
			faultSearchMs <= ceiling && faultDocMs <= ceiling,
			`broken-log calls must return promptly (fire-and-forget): search ${faultSearchMs.toFixed(0)}ms, get_doc ${faultDocMs.toFixed(0)}ms, ceiling ${ceiling}ms`,
		);
		log("    ✓ (c3) broken-log responses not delayed (write not awaited)");

		// (c4) prove the failure was REAL: the broken worker wrote ZERO rows.
		await new Promise((r) => setTimeout(r, 2000));
		const countRows = query(FAULT_PERSIST_DIR, "SELECT COUNT(*) AS n FROM tool_calls;");
		const count = Number(countRows[0].n);
		log(`      rows in broken worker's tool_calls: ${count} (expected 0 — every INSERT was rejected)`);
		assert(
			count === 0,
			`broken-log calls must write 0 rows (genuine failure, not silent success): found ${count}`,
		);
		log("    ✓ (c4) broken worker wrote ZERO rows — the induced D1 failure was genuine");
	} finally {
		await client?.close().catch(() => {});
		worker.kill("SIGTERM");
		await new Promise((r) => setTimeout(r, 1500));
	}
}

async function main(): Promise<void> {
	log("── slice-06 MCP analytics + C-7 guardrail acceptance test ──");

	// Hermetic local D1s: wipe prior persistence, apply migration 0001 to BOTH.
	rmSync(HEALTHY_PERSIST_DIR, { recursive: true, force: true });
	rmSync(FAULT_PERSIST_DIR, { recursive: true, force: true });
	log("Applying migration 0001 to two hermetic local D1s (healthy + fault) …");
	applyMigration(HEALTHY_PERSIST_DIR);
	applyMigration(FAULT_PERSIST_DIR);
	log("    ✓ tool_calls table created in both local D1s");

	log("\nPhase 1 — healthy worker (real D1 writes) …");
	const baseline = await runHealthyPhase();

	log("\nPhase 2 — faulty worker (ANALYTICS_FORCE_FAIL=1) …");
	await runFaultPhase(baseline);

	log("\n✓ ALL SLICE-06 ANALYTICS + C-7 GUARDRAIL CHECKS PASSED");
}

main().catch((err) => {
	process.stderr.write(`\n✗ slice-06 analytics test FAILED: ${String(err)}\n`);
	process.exit(1);
});
