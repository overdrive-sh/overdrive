import { notFound } from "next/navigation";
import { source } from "@/lib/source";
import { getLLMText } from "@/lib/get-llm-text";

// Node runtime, never edge (research § C-2): reads the build-time `source`
// index. OpenNext manages the Worker runtime.
export const runtime = "nodejs";

// Per-page `.md` export handler (US-04 AC: "appending `.md` to a doc URL
// returns that page's clean title + processed markdown").
//
// ROUTING SHAPE — why an API route + rewrite instead of a `[[...slug]].md`
// folder (deviation flagged for slice 05): Next 16 does NOT support a literal
// `.md` suffix on a catch-all segment as a per-page-matching dynamic route. A
// `[[...slug]].md` folder collapses to a single static literal (only the
// empty-slug case prerenders) and a `[...slug].md` folder both (a) fails typed
// routes — its generated `params` type drops `slug` — and (b) registers as a
// static literal that never matches per-page `.md` URLs, so they 404 through
// the greedy `/docs/[[...slug]]` page route under OpenNext/workerd. The robust,
// adapter-agnostic shape is a CLEAN catch-all API route here, fronted by a
// `next.config.ts` rewrite (`/docs/:path*.md` → `/api/md/:path*`). Rewrites run
// at the routing layer that OpenNext honors, and a plain `[[...slug]]` catch-all
// has none of the literal-suffix problems. Slice 05's MCP `get_doc(url)` can
// resolve a page by mapping its url to this same seam — see `lib/get-llm-text.ts`.
//
// `force-dynamic`: rewrite targets render at request time. The corpus is tiny
// and the seam is cheap (one `getText('processed')` read). The `one-index-check`
// build gate (ADR-0058) verifies every page's `.md` is reachable through this
// exact `getLLMText` seam.
export const dynamic = "force-dynamic";

export async function GET(
	_request: Request,
	context: { params: Promise<{ slug?: string[] }> },
): Promise<Response> {
	// `[[...slug]]` is optional: the index page `.md` (`/docs.md` → rewritten to
	// `/api/md`) arrives with no slug; deeper pages arrive with their path.
	const { slug } = await context.params;
	const page = source.getPage(slug);
	if (!page) notFound();

	const body = await getLLMText(page);
	return new Response(body, {
		headers: { "Content-Type": "text/markdown; charset=utf-8" },
	});
}
