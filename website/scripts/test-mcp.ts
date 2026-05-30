/**
 * Slice-05 acceptance test — a REAL MCP-client test (one of the four glue
 * checks the docs-platform feature committed to keep).
 *
 * It spins up the OpenNext Worker on workerd via `wrangler dev`, connects a real
 * MCP `Client` over `StreamableHTTPClientTransport` to `/mcp`, and asserts the
 * US-05 contract end to end:
 *
 *   1. The server lists EXACTLY the two tools `search_docs` + `get_doc`, each
 *      with an input schema.
 *   2. `search_docs(<query matching our DST/intent-observation corpus>)` returns
 *      ranked page results whose top hit is the relevant page.
 *   3. `get_doc(<that url>)` returns clean markdown BYTE-IDENTICAL to the
 *      slice-04 `.md` export for the same page (`GET /docs/<path>.md`) — the
 *      US-05 identity invariant.
 *   4. `get_doc('/docs/does-not-exist')` returns an HONEST not-found, never a
 *      fabricated page.
 *
 * Port-to-port: the test drives the system ONLY through the MCP endpoint (the
 * driving port) and the public `.md` HTTP route, asserting observable outcomes.
 * No internal module is imported. Run with `bun run test:mcp`.
 */
import { spawn, type ChildProcess } from "node:child_process";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const PORT = Number(process.env.MCP_TEST_PORT ?? 8979);
const ORIGIN = `http://127.0.0.1:${PORT}`;

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

async function waitForServer(timeoutMs: number): Promise<void> {
	const deadline = Date.now() + timeoutMs;
	let lastErr: unknown;
	while (Date.now() < deadline) {
		try {
			// The /mcp endpoint rejects a bare GET without the MCP accept header,
			// but any HTTP response (even 4xx) proves the Worker is listening.
			const res = await fetch(`${ORIGIN}/docs.md`);
			if (res.ok || res.status < 500) return;
		} catch (err) {
			lastErr = err;
		}
		await new Promise((r) => setTimeout(r, 500));
	}
	throw new Error(
		`server did not become ready on ${ORIGIN} within ${timeoutMs}ms: ${String(lastErr)}`,
	);
}

async function main(): Promise<void> {
	log("── slice-05 MCP acceptance test ──");
	log(`Starting wrangler dev on :${PORT} (workerd, OpenNext build) …`);

	const wrangler: ChildProcess = spawn(
		"bunx",
		["wrangler", "dev", "--port", String(PORT), "--inspector-port", "0"],
		{ stdio: ["ignore", "inherit", "inherit"], env: process.env },
	);

	const client = new Client({ name: "slice-05-acceptance", version: "0.1.0" });

	try {
		await waitForServer(120_000);
		log("Worker is up. Connecting MCP client to /mcp …");

		const transport = new StreamableHTTPClientTransport(new URL(`${ORIGIN}/mcp`));
		await client.connect(transport);

		// ── Check 1 — exactly the two tools, with schemas ──
		const { tools } = await client.listTools();
		const toolNames = tools.map((t) => t.name).sort();
		log(`\n[1] tools/list → ${JSON.stringify(toolNames)}`);
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

		// ── Check 2 — search_docs top hit is the relevant page ──
		const query = "deterministic simulation";
		const expectedUrl = "/docs/concepts/deterministic-simulation-testing";
		const searchResult = (await client.callTool({
			name: "search_docs",
			arguments: { query },
		})) as CallToolText;
		const pages = JSON.parse(firstText(searchResult)) as {
			title: string;
			url: string;
			excerpt: string;
		}[];
		log(`\n[2] search_docs(${JSON.stringify(query)}) → top hits:`);
		for (const p of pages.slice(0, 3)) log(`      ${p.url}  —  ${p.title}`);
		assert(pages.length > 0, "search_docs returned at least one page");
		assert(
			pages[0].url === expectedUrl,
			`top hit should be ${expectedUrl}, got ${pages[0].url}`,
		);
		assert(
			typeof pages[0].title === "string" && pages[0].title.length > 0,
			"top hit has a title",
		);
		log(`    ✓ top hit is ${pages[0].url} ("${pages[0].title}")`);

		// ── Check 3 — get_doc byte-identical to the .md export ──
		const docResult = (await client.callTool({
			name: "get_doc",
			arguments: { url: expectedUrl },
		})) as CallToolText;
		assert(docResult.isError !== true, "get_doc on a real URL is not an error");
		const docMarkdown = firstText(docResult);
		const mdExport = await (await fetch(`${ORIGIN}${expectedUrl}.md`)).text();
		log(`\n[3] get_doc(${JSON.stringify(expectedUrl)}) identity vs .md export:`);
		log(`      get_doc bytes:   ${docMarkdown.length}`);
		log(`      .md export bytes: ${mdExport.length}`);
		log(`      first line: ${JSON.stringify(docMarkdown.split("\n", 1)[0])}`);
		assert(
			docMarkdown === mdExport,
			"get_doc output must be byte-identical to the .md export (US-05 identity invariant)",
		);
		log("    ✓ get_doc === GET /docs/.../…md  (byte-identical, US-05)");

		// ── Check 4 — honest not-found ──
		const missing = "/docs/does-not-exist";
		const missingResult = (await client.callTool({
			name: "get_doc",
			arguments: { url: missing },
		})) as CallToolText;
		const missingText = firstText(missingResult);
		log(`\n[4] get_doc(${JSON.stringify(missing)}) →`);
		log(`      isError: ${missingResult.isError === true}`);
		log(`      text: ${JSON.stringify(missingText)}`);
		assert(
			missingResult.isError === true,
			"get_doc on a nonexistent URL must report an error (honest not-found)",
		);
		assert(
			!missingText.startsWith("# "),
			"not-found result must NOT be a fabricated page (no '# Title' header)",
		);
		assert(
			/not\s|no\s.*page|does not|doesn't/i.test(missingText),
			"not-found result clearly states the page was not found",
		);
		log("    ✓ honest not-found, no fabricated page");

		await client.close();
		log("\n✓ ALL MCP ACCEPTANCE CHECKS PASSED");
	} finally {
		await client.close().catch(() => {});
		wrangler.kill("SIGTERM");
		// Give workerd a moment to release the port.
		await new Promise((r) => setTimeout(r, 1000));
	}
}

main().catch((err) => {
	process.stderr.write(`\n✗ MCP acceptance test FAILED: ${String(err)}\n`);
	process.exit(1);
});
