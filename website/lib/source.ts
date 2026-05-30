import { blog as blogCollection, docs } from "@/.source/server";
import { loader } from "fumadocs-core/source";
import { toFumadocsSource } from "fumadocs-mdx/runtime/server";

// THE one build-time index (DISCUSS C-4). Every surface that searches or
// exports content — browser search, MCP, llms.txt, blog — is a consumer of
// these `loader()` outputs, never a re-builder. Slice 07 adds a SECOND
// collection (`blog`) as a peer source; the consumers fold both into ONE
// Orama index (`lib/search.ts`), ONE llms export (`lib/get-llm-text.ts` +
// the llms routes), and ONE structural assertion (`scripts/assert-one-index.ts`).
// There is no second index — the blog joins the existing one.
export const source = loader({
	baseUrl: "/docs",
	source: docs.toFumadocsSource(),
});

// The blog source — a flat collection of `doc` posts under `/blog`. Wrapping
// the `defineCollections({ type: 'doc' })` codegen array in `toFumadocsSource`
// gives it the SAME `getPages()` / `getPage([slug])` / `page.data` surface the
// docs `source` exposes, so the index consumers treat it identically. No
// `meta.json` ordering — the list page sorts by `date`.
export const blog = loader({
	baseUrl: "/blog",
	source: toFumadocsSource(blogCollection, []),
});

// ── The published-blog gate (DISCUSS DoR 3rd UAT scenario) ──────────────────
//
// A post with `draft: true` is excluded EVERYWHERE: the `/blog` list, the
// search index, the llms export, the per-page `.md`, and MCP. To keep that
// gate in ONE place, every consumer derives the published set from THIS
// function — never re-implements the `draft` check. `publishedBlogPages()`
// returns the blog pages an unauthenticated visitor (and every agent) may see.
//
// Drafts remain in `blog.getPages()` so the list page can choose to render an
// individual draft post body at `/blog/<draft-slug>` for a logged-out preview
// IF a future slice wants it — but slice 07 does not: the draft is unreachable
// from the list, absent from all indexes, and the post route 404s on it.
export function publishedBlogPages(): ReturnType<typeof blog.getPages> {
	return blog.getPages().filter((page) => page.data.draft !== true);
}
