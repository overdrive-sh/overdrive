import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { getCloudflareContext } from "@opennextjs/cloudflare";
import { z } from "zod";
import { server as searchServer } from "@/lib/search";
import { getLLMText, type LLMTextPage } from "@/lib/get-llm-text";
import { publishedBlogPages, source } from "@/lib/source";

// ── The docs-MCP server (ADR-0055) ──────────────────────────────────────────
//
// A same-Worker Next route handler at `/mcp`. It is a fourth consumer of the
// ONE build-time `source` index (DISCUSS C-4): the two tools delegate to the
// SAME seams the browser search and the `.md`/llms exports already use —
// `lib/search.ts`'s `server.search` (ADR-0057) and `lib/get-llm-text.ts`'s
// `getLLMText` (slice 04). There is no second index and no re-implementation of
// clean-markdown rendering; divergence is structurally impossible.
//
// Transport: `WebStandardStreamableHTTPServerTransport` — the MCP TS SDK's
// Web-Standard (Request/Response/ReadableStream) Streamable-HTTP transport,
// documented for Cloudflare Workers. Because it speaks Web `Request`/`Response`
// natively, it drops straight into a Next route handler with no Node `http`
// shimming (the workerd friction the slice brief flagged never materialises —
// see the slice-05 handoff). STATELESS: `sessionIdGenerator: undefined`, no
// Durable Object, no `McpAgent`, no per-session state — a docs-query server
// needs none (Finding 1.4.2). A fresh server + transport is built per request,
// which is the canonical stateless shape and is safe to do per-isolate.

// Node runtime, NEVER edge (C-2): the in-Worker Orama index + the build-time
// `source` are read in-process. OpenNext manages the Worker runtime.
export const runtime = "nodejs";

// ── Best-effort MCP tool-call analytics (ADR-0056, C-7 guardrail) ────────────
//
// One `{tool, query, ts, result_count}` row per tool call, written to the D1
// `tool_calls` table (migrations/0001_tool_calls.sql). The maintainer reads it
// for KPI-4 (volume) and KPI-5 (top zero-result coverage gaps, J-DOCS-003).
//
// THE C-7 CONTRACT — the single most load-bearing property of this slice:
// the logging path MUST NEVER block, delay, or alter the tool response. It is
// fire-and-forget. `logToolCall` therefore:
//
//   1. Computes NOTHING the response depends on — it receives the already-known
//      `result_count` and returns `void` synchronously.
//   2. Hands the D1 write to `ctx.waitUntil(insert.catch(() => {}))` so the
//      write runs AFTER the response is returned, off the critical path. The
//      response is never awaited against it.
//   3. Catch-swallows everything: a missing binding, a thrown
//      `getCloudflareContext()`, a D1 reject (throttle / no-such-table / malformed
//      row) is, at most, a dropped row — never an error that reaches the handler.
//
// Because the helper itself never throws and never awaits the write, a tool
// callback can call it inline and `return` immediately; the response is byte-
// identical whether the log succeeds, fails, or is never reachable.

type ToolCallRow = {
	tool: "search_docs" | "get_doc";
	query: string;
	resultCount: number;
};

const TOOL_CALLS_TABLE = "tool_calls";

// FAULT-INJECTION SEAM (C-7 test): when env.ANALYTICS_FORCE_FAIL === "1" the
// INSERT targets a table that does not exist, so the live D1 engine rejects the
// statement at run() time with a real `D1_ERROR: no such table` — a genuine
// induced failure, NOT a mocked no-op that secretly succeeds. The catch-swallow
// below must absorb it with zero effect on the response. The flag is set only by
// the acceptance test's broken-binding worker invocation; production never sets it.
function targetTable(forceFail: boolean): string {
	return forceFail ? `${TOOL_CALLS_TABLE}__force_fail_no_such_table` : TOOL_CALLS_TABLE;
}

function logToolCall(row: ToolCallRow): void {
	try {
		const { env, ctx } = getCloudflareContext();
		const db = env.ANALYTICS_DB;
		if (!db) return; // unbound D1 → degrade to "no rows", never "broken tools".

		const forceFail =
			(env as unknown as Record<string, unknown>).ANALYTICS_FORCE_FAIL === "1";
		const table = targetTable(forceFail);
		const ts = Date.now();

		const insert = db
			.prepare(
				`INSERT INTO ${table} (tool, query, ts, result_count) VALUES (?, ?, ?, ?)`,
			)
			.bind(row.tool, row.query, ts, row.resultCount)
			.run();

		// Fire-and-forget: the write outlives the response, errors swallowed.
		ctx.waitUntil(Promise.resolve(insert).catch(() => {}));
	} catch {
		// getCloudflareContext()/binding access threw — drop the row silently.
		// The tool response is unaffected (C-7).
	}
}

// Each request gets a fresh stateless server. No shared mutable state survives a
// request — the transport closes when the response stream ends.
function buildServer(): McpServer {
	const mcp = new McpServer({
		name: "overdrive-docs",
		version: "0.1.0",
	});

	// ── search_docs — page-level ranked results over the ONE index ──
	//
	// Delegates to `searchServer.search(query)` (the SAME `createFromSource`
	// index `/api/search` queries). `server.search` returns `SortedResult[]`
	// mixing `type: 'page' | 'heading' | 'text'`; an agent wants whole pages, so
	// we keep `type === 'page'` rows (their `content` is the page title, `url` is
	// the page URL) and dedupe by URL preserving Orama's rank order. The first
	// matching heading/text under a page supplies a short excerpt.
	mcp.registerTool(
		"search_docs",
		{
			title: "Search Overdrive docs",
			description:
				"Full-text search over the Overdrive documentation. Returns ranked pages " +
				"(title, url, excerpt) most relevant to the query. Follow up with get_doc(url) " +
				"to read a page's full clean markdown.",
			inputSchema: { query: z.string().min(1).describe("Search query") },
		},
		async ({ query }) => {
			const results = await searchServer.search(query);

			// First non-page snippet per page URL → excerpt.
			const excerptByUrl = new Map<string, string>();
			for (const result of results) {
				if (result.type === "page") continue;
				if (!excerptByUrl.has(result.url)) {
					excerptByUrl.set(result.url, result.content);
				}
			}

			const seen = new Set<string>();
			const pages: { title: string; url: string; excerpt: string }[] = [];
			for (const result of results) {
				if (result.type !== "page") continue;
				if (seen.has(result.url)) continue;
				seen.add(result.url);
				pages.push({
					title: result.content,
					url: result.url,
					excerpt: excerptByUrl.get(result.url) ?? "",
				});
			}

			// result_count = number of page results (0 ⇒ coverage-gap signal, KPI-5).
			// Best-effort log scheduled here, off the response's critical path (C-7):
			// the response object below is returned regardless of the write outcome.
			logToolCall({ tool: "search_docs", query, resultCount: pages.length });

			return {
				content: [{ type: "text", text: JSON.stringify(pages, null, 2) }],
				structuredContent: { results: pages },
			};
		},
	);

	// ── get_doc — clean per-page markdown, byte-identical to the `.md` export ──
	//
	// Resolves a docs OR published-blog URL to its page and returns
	// `getLLMText(page)` — the EXACT function the `.md` export routes return, so
	// `get_doc(url)` output is byte-identical to `GET <url>.md` (US-05 identity
	// invariant) for both `/docs/...` and `/blog/...`. A `/docs` URL maps through
	// `source.getPage`; a `/blog/<slug>` URL maps through `publishedBlogPages()`
	// — the SAME published set the blog `.md` route and search use, so a draft
	// post (DoR 3rd UAT scenario) resolves to no page and returns an HONEST
	// not-found (`isError: true`), never fabricated content.
	mcp.registerTool(
		"get_doc",
		{
			title: "Read an Overdrive doc or blog page",
			description:
				"Fetch the full clean markdown of a single Overdrive documentation or blog " +
				"page by its URL (e.g. /docs/concepts/intent-observation or /blog/why-overdrive). " +
				"Returns the same content as the page's .md export. Use search_docs first to " +
				"find a page URL.",
			inputSchema: {
				url: z
					.string()
					.min(1)
					.describe(
						"Doc or blog page URL, e.g. /docs/concepts/intent-observation or /blog/why-overdrive",
					),
			},
		},
		async ({ url }) => {
			const page = resolvePage(url);
			if (!page) {
				// result_count = 0: the requested url resolved to no page.
				logToolCall({ tool: "get_doc", query: url, resultCount: 0 });
				return {
					content: [
						{
							type: "text",
							text: `No Overdrive doc or blog page resolves to URL "${url}". Use search_docs to find a valid page URL.`,
						},
					],
					isError: true,
				};
			}

			const markdown = await getLLMText(page);
			// result_count = 1: the url resolved to exactly one page.
			// Scheduled off the critical path (C-7) before the response returns.
			logToolCall({ tool: "get_doc", query: url, resultCount: 1 });
			return { content: [{ type: "text", text: markdown }] };
		},
	);

	return mcp;
}

// Resolves a docs OR published-blog URL to its page. A `/blog/<slug>` URL (with
// an optional `.md` suffix or absent leading slash) resolves only against the
// PUBLISHED blog set (`publishedBlogPages()` — the single draft gate), so a
// draft post returns `undefined` here and `get_doc` answers an honest
// not-found. Everything else falls through to the docs `source.getPage` slug
// mapping. Returned pages satisfy `LLMTextPage`, the seam `getLLMText` reads.
function resolvePage(url: string): LLMTextPage | undefined {
	let path = url.trim();
	if (path.endsWith(".md")) path = path.slice(0, -".md".length);
	if (!path.startsWith("/")) path = `/${path}`;
	if (path === "/blog" || path === "/blog/") {
		// The blog list page itself is not an LLM-text page (no MDX body).
		return undefined;
	}
	if (path.startsWith("/blog/")) {
		return publishedBlogPages().find((page) => page.url === path);
	}
	return source.getPage(slugFromDocUrl(url)) ?? undefined;
}

// Maps a doc URL to the `source.getPage` slug. `/docs/a/b` → `["a","b"]`;
// `/docs` (with or without trailing slash) → `[]` (the index page). Tolerates an
// absent leading slash and a stray `.md` suffix so an agent that passes the
// machine-readable URL still resolves. Mirrors the `.md` route's slug handling
// (`lib/get-llm-text.ts`) — do NOT re-derive the route shape elsewhere.
function slugFromDocUrl(url: string): string[] {
	let path = url.trim();
	if (path.endsWith(".md")) path = path.slice(0, -".md".length);
	// Normalise to a `/docs`-rooted path regardless of leading slash.
	if (!path.startsWith("/")) path = `/${path}`;
	if (path === "/docs" || path === "/docs/") return [];
	const prefix = "/docs/";
	if (!path.startsWith(prefix)) {
		// Not a /docs URL — return the raw segments so getPage misses honestly.
		return path.split("/").filter(Boolean);
	}
	return path.slice(prefix.length).split("/").filter(Boolean);
}

async function handle(request: Request): Promise<Response> {
	// Stateless: fresh server + transport per request, no session id.
	//
	// `enableJsonResponse: true` is load-bearing on workerd (the key slice-05
	// transport finding). By default the transport answers a POST by opening an
	// SSE `ReadableStream` and keeps a standalone GET SSE stream alive — a
	// long-lived-connection model that suits a stateful Node server but that
	// workerd, with its per-request lifecycle, flags as "your Worker's code had
	// hung and would never generate a response." A docs-query server is pure
	// request/response with no server-initiated notifications, so JSON-response
	// mode (one JSON body per POST, GET reduced to a 405) is both correct and
	// hang-free under the Workers runtime.
	const transport = new WebStandardStreamableHTTPServerTransport({
		sessionIdGenerator: undefined,
		enableJsonResponse: true,
	});
	const mcp = buildServer();
	await mcp.connect(transport);
	const response = await transport.handleRequest(request);
	// JSON-response mode buffers the body fully before `handleRequest` resolves,
	// so tearing down the per-request server + transport here releases the
	// transport's stream-lifecycle bookkeeping inside the same request rather
	// than letting it outlive the response (which workerd flags as a hung
	// request). Best-effort: a close failure must not mask a good response.
	await mcp.close().catch(() => {});
	return response;
}

export const POST = handle;
export const GET = handle;
export const DELETE = handle;
