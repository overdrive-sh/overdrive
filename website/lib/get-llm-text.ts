import type { source } from "@/lib/source";

// ── The ONE clean-markdown content seam (DESIGN driven-port, slice 04) ──
//
// `getLLMText(page)` is the SINGLE definition that turns a build-time `source`
// page into clean, agent-consumable markdown: a `# Title (url)` header followed
// by `getText('processed')` — the chrome-free MDAST-rendered markdown that
// `source.config.ts`'s `includeProcessedMarkdown: true` produces. No HTML
// chrome (`<aside>`, `<nav>`, `data-sidebar`, …) ever appears in this output —
// `processed` is markdown, not the rendered React page.
//
// Seam contract (load-bearing across slices):
//
//   • `/llms-full.txt`           maps EVERY `source.getPages()` page through
//                                this function and joins them.
//   • per-page `.md` route       returns this function's output for one page.
//   • slice 05's MCP `get_doc`   MUST import and call THIS exact function so
//     (future)                   `get_doc(url)` output is BYTE-IDENTICAL to the
//                                `.md` export for the same page (US-05 identity
//                                invariant — enforced structurally by
//                                `scripts/assert-one-index.ts`). Do NOT
//                                re-implement title+processed concatenation
//                                anywhere else; that would let the two surfaces
//                                drift. This module is the single source of
//                                clean per-page markdown.
//
// `.md` URL pattern (so slice 05's `get_doc` can resolve a page from a URL):
// a doc page at `page.url` (e.g. `/docs/concepts/intent-observation`) is
// reachable as machine-readable markdown by appending `.md`
// (`/docs/concepts/intent-observation.md`). The index page `/docs` is
// reachable at `/docs.md` (handled by the `[[...slug]]` catch-all + the
// `index.mdx` → `/docs` url mapping). The route handler maps the request
// pathname back to a page via `source.getPage(slug)`.
export async function getLLMText(
	page: (typeof source)["$inferPage"],
): Promise<string> {
	const processed = await page.data.getText("processed");
	return `# ${page.data.title} (${page.url})\n\n${processed}`;
}
