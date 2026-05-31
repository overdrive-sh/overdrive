import {
	type AdvancedIndex,
	createSearchAPI,
	type SearchAPI,
} from "fumadocs-core/search/server";
import { publishedBlogPages, source } from "@/lib/source";

// ── The ONE-index search seam (ADR-0057; extended in slice 07 for the blog) ──
//
// THE C-4 INVARIANT: there is exactly ONE Orama search index, and it covers
// docs AND published blog posts — not a docs index plus a separate blog index.
//
// In slices 03–06 this was `createFromSource(source)`: that helper maps a
// SINGLE loader's `getPages()` to per-page index documents and feeds them to
// `initAdvancedSearch` — one Orama DB over one source. Slice 07 adds a second
// source (the blog). `createFromSource` takes only one loader, so calling it
// twice would build TWO indexes — exactly the divergence C-4 forbids. Instead
// we build the per-page index documents from BOTH sources and hand the combined
// list to ONE `createSearchAPI('advanced')` call — the same advanced Orama index
// `createFromSource` produces, now spanning two collections. One DB, one ranking
// path, one `search`/`GET` surface.
//
// The returned `server` exposes BOTH transports over that one index:
//
//   • `server.GET(request)`        — HTTP handler. `app/api/search/route.ts`
//                                    re-exports it for the browser Cmd+K dialog.
//   • `server.search(query, opts)` — programmatic `(query) => SortedResult[]`.
//                                    MCP `search_docs` (slice 05) imports
//                                    `server` and calls `server.search(query)`.
//
// DRAFT EXCLUSION lives in ONE place: `publishedBlogPages()` (lib/source.ts).
// A draft post is never added to `blogIndexes`, so it is unreachable from search
// AND — because MCP `search_docs` queries this same `server` — from MCP.
//
// Seam contract: import { server } from "@/lib/search". Do NOT build a second
// index anywhere; this module is the single initialization site.

// The minimal structural shape `indexDocument` reads. `structuredData` is
// value-or-thunk: the docs collection exposes it as a value, the blog `doc`
// collection (via `toFumadocsSource`) as `() => Promise<StructuredData>` —
// exactly the value-or-function `buildIndexDefault` handles internally. Typing
// the input on this shape lets the ONE builder serve BOTH sources.
type IndexablePage = {
	url: string;
	data: {
		title: string;
		description?: string;
		structuredData: AdvancedIndex["structuredData"] | (() => Promise<AdvancedIndex["structuredData"]>);
	};
};

// Build one Orama `AdvancedIndex` document per page from PUBLIC page data —
// `page.data.structuredData` is the same field `createFromSource` reads. This
// keeps the combined index byte-identical in shape to the per-source index the
// docs already used, without depending on any fumadocs internal helper.
async function indexDocument(page: IndexablePage): Promise<AdvancedIndex> {
	const raw = page.data.structuredData;
	const structuredData = typeof raw === "function" ? await raw() : raw;
	return {
		id: page.url,
		title: page.data.title,
		description: page.data.description,
		url: page.url,
		structuredData,
	};
}

// Build the ONE index lazily from a function so the loaders are read once when
// the first query arrives — inside the Next build / Worker runtime, where the
// fumadocs-mdx loader has resolved `structuredData`.
export const server: SearchAPI = createSearchAPI("advanced", {
	indexes: async () => {
		const docsIndexes = await Promise.all(source.getPages().map(indexDocument));
		// PUBLISHED blog posts only — the draft gate (DoR 3rd UAT scenario).
		const blogIndexes = await Promise.all(
			publishedBlogPages().map(indexDocument),
		);
		return [...docsIndexes, ...blogIndexes];
	},
});
