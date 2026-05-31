/**
 * Slice-07 acceptance test — the blog as a SECOND collection joining the ONE
 * index, with drafts excluded everywhere (DISCUSS C-4 + the DoR 3rd UAT
 * scenario).
 *
 * It runs the REAL OpenNext Worker on workerd via `wrangler dev`, drives the
 * public HTTP surface (`/blog`, `/blog/<slug>`, `/api/search`, `/llms.txt`,
 * per-page `.md`) AND a real MCP `Client` over `StreamableHTTPClientTransport`
 * to `/mcp`, and asserts:
 *
 *   [1] GET /blog → 200, lists the PUBLISHED posts (title + date); the DRAFT
 *       post is NOT listed.
 *   [2] GET /blog/<published-slug> → 200, renders the post body.
 *   [3] JOIN PROOF — a term UNIQUE to a published blog post is returned by BOTH
 *       /api/search AND MCP search_docs (top/near-top hit = that blog post URL),
 *       proving the blog joined the one index. get_doc(<blog url>) returns its
 *       clean markdown, byte-identical to the post's .md export.
 *   [4] DRAFT EXCLUSION — a term unique to the DRAFT post returns NOTHING from
 *       /api/search, MCP search_docs, and /llms.txt; GET /blog/<draft-slug> is
 *       404 (drafts unreachable, per the documented design choice).
 *   [5] (covered by `bun run assert:one-index` — run separately in the gate.)
 *   [6] Slice-05/06 invariants still hold — exactly [get_doc, search_docs] with
 *       schemas; docs search top-hit unchanged; docs get_doc === .md identity;
 *       honest not-found.
 *
 * Port-to-port: the system is driven ONLY through the public HTTP routes and the
 * MCP endpoint (driving ports); no internal module is imported.
 *
 * Run with `bun run test:blog`.
 */
import { spawn, type ChildProcess } from "node:child_process";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const PORT = Number(process.env.BLOG_TEST_PORT ?? 8991);
const ORIGIN = `http://127.0.0.1:${PORT}`;

// Fixtures (content/blog/*.mdx).
const PUBLISHED_URL = "/blog/how-fumadocs-powers-these-docs";
const SECOND_PUBLISHED_URL = "/blog/why-overdrive";
const PUBLISHED_TITLE = "How Fumadocs powers these docs";
const DRAFT_URL = "/blog/upcoming-roadmap-draft";
// A coined term that appears ONLY in the published post body, nowhere in docs —
// tokenizer-safe (a single alphabetic word), so the join proof is unambiguous.
const PUBLISHED_UNIQUE_TERM = "indexconvergence";
// A term that appears ONLY in the draft post.
const DRAFT_UNIQUE_TERM = "xyzzyplughdraftmarker";

type CallToolText = {
	content: { type: string; text?: string }[];
	isError?: boolean;
};

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

type SearchHit = { type: string; url: string; content: string };

async function httpSearch(query: string): Promise<SearchHit[]> {
	const res = await fetch(
		`${ORIGIN}/api/search?query=${encodeURIComponent(query)}`,
	);
	assert(res.ok, `/api/search?query=${query} → ${res.status}`);
	return (await res.json()) as SearchHit[];
}

function pageUrls(hits: SearchHit[]): string[] {
	const seen: string[] = [];
	for (const hit of hits) {
		if (hit.type === "page" && !seen.includes(hit.url)) seen.push(hit.url);
	}
	return seen;
}

async function mcpSearchPages(
	client: Client,
	query: string,
): Promise<{ title: string; url: string; excerpt: string }[]> {
	const result = (await client.callTool({
		name: "search_docs",
		arguments: { query },
	})) as CallToolText;
	return JSON.parse(firstText(result)) as {
		title: string;
		url: string;
		excerpt: string;
	}[];
}

async function main(): Promise<void> {
	log("── slice-07 blog acceptance test ──");
	log(`Starting wrangler dev on :${PORT} (workerd, OpenNext build) …`);

	const wrangler: ChildProcess = spawn(
		"bunx",
		["wrangler", "dev", "--port", String(PORT), "--inspector-port", "0"],
		{ stdio: ["ignore", "inherit", "inherit"], env: process.env },
	);

	const client = new Client({ name: "slice-07-acceptance", version: "0.1.0" });

	try {
		await waitForServer(120_000);
		log("Worker is up. Connecting MCP client to /mcp …");
		const transport = new StreamableHTTPClientTransport(
			new URL(`${ORIGIN}/mcp`),
		);
		await client.connect(transport);

		// ── [1] /blog lists published posts, draft absent ──
		const listRes = await fetch(`${ORIGIN}/blog`);
		assert(listRes.ok, `GET /blog → ${listRes.status}`);
		const listHtml = await listRes.text();
		log(`\n[1] GET /blog → ${listRes.status}`);
		assert(
			listHtml.includes(PUBLISHED_URL),
			`/blog lists published post ${PUBLISHED_URL}`,
		);
		assert(
			listHtml.includes(SECOND_PUBLISHED_URL),
			`/blog lists published post ${SECOND_PUBLISHED_URL}`,
		);
		assert(
			listHtml.includes(PUBLISHED_TITLE),
			`/blog shows the published post title`,
		);
		assert(
			!listHtml.includes(DRAFT_URL),
			`/blog must NOT link the draft post ${DRAFT_URL}`,
		);
		assert(
			!listHtml.includes(DRAFT_UNIQUE_TERM),
			`/blog must NOT surface the draft's unique term`,
		);
		log(`    ✓ lists ${PUBLISHED_URL} + ${SECOND_PUBLISHED_URL}; draft absent`);

		// ── [2] /blog/<published> renders the post body ──
		const postRes = await fetch(`${ORIGIN}${PUBLISHED_URL}`);
		assert(postRes.ok, `GET ${PUBLISHED_URL} → ${postRes.status}`);
		const postHtml = await postRes.text();
		log(`\n[2] GET ${PUBLISHED_URL} → ${postRes.status}`);
		assert(postHtml.includes(PUBLISHED_TITLE), "post page renders the title");
		assert(
			postHtml.includes("One index, four consumers"),
			"post page renders a body heading (the MDX rendered)",
		);
		log("    ✓ post body renders");

		// ── [3] JOIN PROOF — published term in /api/search AND MCP ──
		log(
			`\n[3] join proof — unique term ${JSON.stringify(PUBLISHED_UNIQUE_TERM)}`,
		);
		const httpHits = pageUrls(await httpSearch(PUBLISHED_UNIQUE_TERM));
		log(`    /api/search page hits: ${JSON.stringify(httpHits.slice(0, 3))}`);
		assert(
			httpHits.includes(PUBLISHED_URL),
			`/api/search must return ${PUBLISHED_URL} for a term unique to it`,
		);
		const mcpHits = await mcpSearchPages(client, PUBLISHED_UNIQUE_TERM);
		const mcpUrls = mcpHits.map((p) => p.url);
		log(`    MCP search_docs page hits: ${JSON.stringify(mcpUrls.slice(0, 3))}`);
		assert(
			mcpUrls.includes(PUBLISHED_URL),
			`MCP search_docs must return ${PUBLISHED_URL} for the same term`,
		);
		assert(
			mcpUrls[0] === PUBLISHED_URL,
			`MCP top hit should be the blog post (${PUBLISHED_URL}), got ${mcpUrls[0]}`,
		);
		log(`    ✓ blog post joined the ONE index (search + MCP both find it)`);

		// get_doc(<blog url>) === the post's .md export.
		const docResult = (await client.callTool({
			name: "get_doc",
			arguments: { url: PUBLISHED_URL },
		})) as CallToolText;
		assert(docResult.isError !== true, "get_doc(blog url) is not an error");
		const docMarkdown = firstText(docResult);
		const mdExport = await (await fetch(`${ORIGIN}${PUBLISHED_URL}.md`)).text();
		log(`    get_doc bytes: ${docMarkdown.length}  .md bytes: ${mdExport.length}`);
		assert(
			docMarkdown === mdExport,
			"get_doc(blog url) byte-identical to the blog post's .md export",
		);
		assert(
			docMarkdown.startsWith(`# ${PUBLISHED_TITLE} (${PUBLISHED_URL})`),
			"get_doc(blog url) carries the clean title header",
		);
		log("    ✓ get_doc(blog) === blog .md export (byte-identical)");

		// ── [4] DRAFT EXCLUSION everywhere ──
		log(`\n[4] draft exclusion — unique term ${JSON.stringify(DRAFT_UNIQUE_TERM)}`);
		const draftHttpHits = pageUrls(await httpSearch(DRAFT_UNIQUE_TERM));
		assert(
			!draftHttpHits.includes(DRAFT_URL) && draftHttpHits.length === 0,
			`/api/search must return NOTHING for the draft's unique term, got ${JSON.stringify(
				draftHttpHits,
			)}`,
		);
		const draftMcpHits = await mcpSearchPages(client, DRAFT_UNIQUE_TERM);
		assert(
			!draftMcpHits.some((p) => p.url === DRAFT_URL),
			`MCP search_docs must NOT return the draft post`,
		);
		const llms = await (await fetch(`${ORIGIN}/llms.txt`)).text();
		assert(
			!llms.includes(DRAFT_URL) && !llms.includes(DRAFT_UNIQUE_TERM),
			"/llms.txt must NOT contain the draft URL or its unique term",
		);
		// And the draft post page itself is unreachable.
		const draftPageRes = await fetch(`${ORIGIN}${DRAFT_URL}`);
		log(`    GET ${DRAFT_URL} → ${draftPageRes.status}`);
		assert(
			draftPageRes.status === 404,
			`GET ${DRAFT_URL} must be 404 (drafts unreachable), got ${draftPageRes.status}`,
		);
		// get_doc on the draft is an honest not-found, not fabricated content.
		const draftDoc = (await client.callTool({
			name: "get_doc",
			arguments: { url: DRAFT_URL },
		})) as CallToolText;
		assert(
			draftDoc.isError === true,
			"get_doc(draft url) must be an honest not-found",
		);
		log("    ✓ draft absent from search, MCP, llms.txt; 404 page; honest get_doc miss");

		// ── [6] slice-05/06 invariants still hold ──
		log("\n[6] slice-05/06 invariants");
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
		// Docs search top-hit unchanged.
		const docsQuery = "deterministic simulation";
		const docsExpected = "/docs/concepts/deterministic-simulation-testing";
		const docsMcp = await mcpSearchPages(client, docsQuery);
		assert(
			docsMcp.length > 0 && docsMcp[0].url === docsExpected,
			`docs search top hit should be ${docsExpected}, got ${docsMcp[0]?.url}`,
		);
		// Docs get_doc === .md identity.
		const docsDoc = (await client.callTool({
			name: "get_doc",
			arguments: { url: docsExpected },
		})) as CallToolText;
		const docsMd = await (await fetch(`${ORIGIN}${docsExpected}.md`)).text();
		assert(
			firstText(docsDoc) === docsMd,
			"docs get_doc === .md identity still holds",
		);
		// Honest not-found.
		const missing = (await client.callTool({
			name: "get_doc",
			arguments: { url: "/docs/does-not-exist" },
		})) as CallToolText;
		assert(missing.isError === true, "honest not-found still holds");
		log("    ✓ tools, docs top-hit, docs get_doc identity, honest not-found");

		await client.close();
		log("\n✓ ALL BLOG ACCEPTANCE CHECKS PASSED");
	} finally {
		await client.close().catch(() => {});
		wrangler.kill("SIGTERM");
		await new Promise((r) => setTimeout(r, 1000));
	}
}

main().catch((err) => {
	process.stderr.write(`\n✗ blog acceptance test FAILED: ${String(err)}\n`);
	process.exit(1);
});
