import { publishedBlogPages, source } from "@/lib/source";
import { getLLMText } from "@/lib/get-llm-text";

// Node runtime, never edge (research § C-2): reads the build-time `source`
// index. OpenNext manages the Worker runtime.
export const runtime = "nodejs";

// `/llms-full.txt` — the WHOLE corpus as one clean-markdown document (a
// consumer of the ONE build-time index; DISCUSS C-4). Every docs page AND every
// PUBLISHED blog post is mapped through `lib/get-llm-text.ts` — the single
// clean-markdown seam — and the results joined. The output is markdown headings
// + prose, NO HTML chrome (no `<aside>`, no `<nav>`, no `data-sidebar`), because
// `getText('processed')` renders MDAST, not the React page.
//
// DRAFT EXCLUSION (DoR 3rd UAT scenario): blog pages come from
// `publishedBlogPages()` — a `draft: true` post is never appended, so its body
// never reaches an agent through this export.
export async function GET(): Promise<Response> {
	const pages = [...source.getPages(), ...publishedBlogPages()];
	const scan = await Promise.all(pages.map((page) => getLLMText(page)));
	const body = scan.join("\n\n");
	return new Response(body, {
		headers: { "Content-Type": "text/plain; charset=utf-8" },
	});
}
