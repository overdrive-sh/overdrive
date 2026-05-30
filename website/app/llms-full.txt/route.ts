import { source } from "@/lib/source";
import { getLLMText } from "@/lib/get-llm-text";

// Node runtime, never edge (research § C-2): reads the build-time `source`
// index. OpenNext manages the Worker runtime.
export const runtime = "nodejs";

// `/llms-full.txt` — the WHOLE corpus as one clean-markdown document (fourth
// consumer of the ONE build-time `source` index; DISCUSS C-4). Every
// `source.getPages()` page is mapped through `lib/get-llm-text.ts` — the single
// clean-markdown seam — and the results joined. The output is markdown headings
// + prose, NO HTML chrome (no `<aside>`, no `<nav>`, no `data-sidebar`), because
// `getText('processed')` renders MDAST, not the React page.
export async function GET(): Promise<Response> {
	const pages = source.getPages();
	const scan = await Promise.all(pages.map((page) => getLLMText(page)));
	const body = scan.join("\n\n");
	return new Response(body, {
		headers: { "Content-Type": "text/plain; charset=utf-8" },
	});
}
