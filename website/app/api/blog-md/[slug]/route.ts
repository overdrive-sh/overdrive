import { notFound } from "next/navigation";
import { publishedBlogPages } from "@/lib/source";
import { getLLMText } from "@/lib/get-llm-text";

// Node runtime, never edge (research § C-2): reads the build-time `blog`
// source. OpenNext manages the Worker runtime.
export const runtime = "nodejs";

// Per-post `.md` export for the blog (slice 07 — the blog half of US-04's
// "appending `.md` to a page URL returns clean title + processed markdown").
//
// ROUTING SHAPE — mirrors the docs `.md` route (`app/api/md/[[...slug]]`): Next
// 16 cannot express a literal `.md` suffix on a route segment, so a
// `next.config.ts` rewrite (`/blog/:slug.md` → `/api/blog-md/:slug`) points
// every blog `.md` URL at this clean API route, which runs the SAME `getLLMText`
// seam the docs `.md`, llms-full, and MCP `get_doc` use (no second clean-
// markdown path; C-4). Blog posts are flat, so `[slug]` is a single segment, not
// a catch-all.
//
// DRAFT EXCLUSION (DoR 3rd UAT scenario): the page is resolved from
// `publishedBlogPages()` — the single draft gate — NOT `blog.getPage()`. A
// `draft: true` post 404s here, so its body is never reachable as `.md` and
// MCP `get_doc(/blog/<draft>)` (which resolves through the same published set)
// returns an honest not-found.
//
// `force-dynamic`: rewrite targets render at request time; the corpus is tiny.
export const dynamic = "force-dynamic";

export async function GET(
	_request: Request,
	context: { params: Promise<{ slug: string }> },
): Promise<Response> {
	const { slug } = await context.params;
	const page = publishedBlogPages().find(
		(candidate) => candidate.url === `/blog/${slug}`,
	);
	// Drafts (and unknown slugs) are not in the published set — honest 404.
	if (!page) notFound();

	const body = await getLLMText(page);
	return new Response(body, {
		headers: { "Content-Type": "text/markdown; charset=utf-8" },
	});
}
