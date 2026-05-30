import { llms } from "fumadocs-core/source";
import { publishedBlogPages, source } from "@/lib/source";

// Node runtime, never edge (research § C-2): reads the build-time `source`
// index. OpenNext manages the Worker runtime.
export const runtime = "nodejs";

// `/llms.txt` — the corpus INDEX (a consumer of the ONE build-time index;
// DISCUSS C-4). `llms(source).index()` renders a markdown index of every doc
// URL; slice 07 appends a `## Blog` section listing every PUBLISHED blog post
// URL. A page missing here is caught by `scripts/assert-one-index.ts`
// (consumer #2 check, ADR-0058).
//
// DRAFT EXCLUSION (DoR 3rd UAT scenario): blog URLs come from
// `publishedBlogPages()` — the single draft gate. A `draft: true` post's URL
// never appears in this index, so it is not advertised to any agent.
//
// Why a hand-rolled blog section rather than `llms(blog).index()`: the blog is
// flat with no `meta.json`, and we must filter to published posts. Listing the
// published URLs directly keeps the draft gate in ONE place (`publishedBlogPages`)
// and keeps the URL-presence assertion falsifiable.
export function GET(): Response {
	const docsIndex = llms(source).index();
	const blogLines = publishedBlogPages()
		.map((page) => {
			const description = page.data.description ?? page.data.summary ?? "";
			const suffix = description ? `: ${description}` : "";
			return `- [${page.data.title}](${page.url})${suffix}`;
		})
		.join("\n");
	const body = blogLines
		? `${docsIndex}\n\n## Blog\n\n${blogLines}\n`
		: docsIndex;
	return new Response(body, {
		headers: { "Content-Type": "text/plain; charset=utf-8" },
	});
}
