import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { WebStandardStreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js";
import { z } from "zod";
import { server as searchServer } from "@/lib/search";
import { getLLMText } from "@/lib/get-llm-text";
import { source } from "@/lib/source";

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

			return {
				content: [{ type: "text", text: JSON.stringify(pages, null, 2) }],
				structuredContent: { results: pages },
			};
		},
	);

	// ── get_doc — clean per-page markdown, byte-identical to the `.md` export ──
	//
	// Resolves a doc URL to a `source` page and returns `getLLMText(page)` — the
	// EXACT function `app/api/md/[[...slug]]/route.ts` returns, so `get_doc(url)`
	// output is byte-identical to `GET /docs/<path>.md` (US-05 identity
	// invariant). URL → slug: strip the `/docs` base, split on `/`; `/docs`
	// itself maps to the empty slug (the index page). A URL that does not resolve
	// returns an HONEST not-found (`isError: true`), never fabricated content.
	mcp.registerTool(
		"get_doc",
		{
			title: "Read an Overdrive doc page",
			description:
				"Fetch the full clean markdown of a single Overdrive documentation page by " +
				"its URL (e.g. /docs/concepts/intent-observation). Returns the same content " +
				"as the page's .md export. Use search_docs first to find a page URL.",
			inputSchema: {
				url: z
					.string()
					.min(1)
					.describe("Doc page URL, e.g. /docs/concepts/intent-observation"),
			},
		},
		async ({ url }) => {
			const page = source.getPage(slugFromDocUrl(url));
			if (!page) {
				return {
					content: [
						{
							type: "text",
							text: `No Overdrive doc page resolves to URL "${url}". Use search_docs to find a valid page URL.`,
						},
					],
					isError: true,
				};
			}

			const markdown = await getLLMText(page);
			return { content: [{ type: "text", text: markdown }] };
		},
	);

	return mcp;
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
