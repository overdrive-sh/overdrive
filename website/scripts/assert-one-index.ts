import { llms } from "fumadocs-core/source";
import { source } from "@/lib/source";
import { getLLMText } from "@/lib/get-llm-text";
import { server } from "@/lib/search";

// ── The C-4 / KPI-3 one-index structural assertion (ADR-0058) ──
//
// C-4 is the strategic invariant of the docs-platform feature: "search, MCP,
// and llms export all consume the SAME build-time `source` index; no parallel
// indexes." ADR-0055 (same-Worker MCP) and ADR-0057 (`lib/search.ts` seam) make
// divergence structurally *unlikely*; this assertion makes the invariant
// *verified* on every build (Earned Trust, nWave principle 12). It is the
// website's analogue of the Rust dst-lint gate.
//
// It enumerates the corpus ONCE from `source.getPages()` — so it cannot itself
// introduce a second source of truth — and checks every page against the three
// (today) consumers of that one index:
//
//   1. `.md` export       — `getLLMText(page)` resolves and is non-empty (the
//                           per-page `app/docs/[[...slug]].md` route reads this
//                           exact seam; US-04 AC "100% of pages have a reachable
//                           `.md`").
//   2. `llms.txt`         — the page's URL appears in `llms(source).index()`
//                           (US-04 AC — no page silently omitted from the index).
//   3. search index       — `server.search(<title>)` (the SAME Orama index the
//                           browser Cmd+K and slice 05's MCP `search_docs`
//                           query) returns a `type: 'page'` result whose `url`
//                           equals `page.url` (C-4 — a page in the corpus but
//                           absent from search would be invisible to humans and
//                           agents).
//
// Two of the three checks (`getText('processed')` for the `.md` export, and the
// Orama index build behind `server.search`) require the fumadocs-mdx LOADER to
// have resolved each page's compiled content + structured data. That resolution
// only happens inside the Next/Turbopack build — a bare `bun run` of this file
// imports the raw `.source` module WITHOUT the loader and `getText('processed')`
// / `server.search` throw. So the assertion is driven from a force-static route
// (`app/__assert-one-index/route.ts`) that Next prerenders during `next build`:
// a violation throws there and fails the build, which is exactly the ADR-0058
// "wired into the build pipeline so a failure fails the build" contract. See
// that route's `bun run assert:one-index` deviation note.
//
// When a NEW consumer of the one index is added (blog joins in slice 07; MCP in
// slice 05 reuses these same seams), it gains a corresponding check here in the
// same change — per ADR-0058's Earned-Trust self-application clause.

export interface OneIndexViolation {
	url: string;
	title: string;
	consumer: "md-export" | "llms.txt" | "search-index";
	detail: string;
}

export interface OneIndexResult {
	pageCount: number;
	violations: OneIndexViolation[];
}

// Runs every consumer check over the single `source.getPages()` enumeration.
// Pure data-in/data-out — the caller decides how to surface failure (throw in a
// build route, exit non-zero in a CLI). MUST be called in a context where the
// fumadocs-mdx loader has resolved page content (i.e. inside the Next build /
// runtime), not bare bun.
export async function checkOneIndex(): Promise<OneIndexResult> {
	const pages = source.getPages();
	const violations: OneIndexViolation[] = [];

	// Build each consumer's view ONCE from the single enumeration.
	const llmsIndex = llms(source).index();

	for (const page of pages) {
		const title = page.data.title ?? "(untitled)";

		// Consumer 1 — reachable, non-empty `.md` export via the shared seam.
		let mdText = "";
		try {
			mdText = await getLLMText(page);
		} catch (error) {
			violations.push({
				url: page.url,
				title,
				consumer: "md-export",
				detail: `getLLMText threw: ${(error as Error).message}`,
			});
		}
		if (mdText.trim().length === 0) {
			violations.push({
				url: page.url,
				title,
				consumer: "md-export",
				detail: "getLLMText resolved to empty text",
			});
		}

		// Consumer 2 — present in the llms.txt index.
		if (!llmsIndex.includes(page.url)) {
			violations.push({
				url: page.url,
				title,
				consumer: "llms.txt",
				detail: "page URL not found in llms(source).index() output",
			});
		}

		// Consumer 3 — present in the search index (queried through the SAME
		// seam the browser + MCP use). A `type: 'page'` hit whose url == page.url
		// proves the page document was indexed.
		const results = await server.search(title);
		const inIndex = results.some(
			(result) => result.type === "page" && result.url === page.url,
		);
		if (!inIndex) {
			violations.push({
				url: page.url,
				title,
				consumer: "search-index",
				detail: `server.search(${JSON.stringify(
					title,
				)}) returned no page-result for this URL`,
			});
		}
	}

	return { pageCount: pages.length, violations };
}

// Formats a passing/failing summary line set. The build route logs these and
// throws on failure; a future CLI runner can reuse the same formatting.
export function formatOneIndexResult(result: OneIndexResult): string[] {
	if (result.violations.length === 0) {
		return [
			`✓ one-index assertion PASSED — all ${result.pageCount} page(s) reachable from all 3 consumers (.md export, llms.txt, search index). C-4 verified (ADR-0058).`,
		];
	}
	const lines = [
		`✗ one-index assertion FAILED — ${result.violations.length} violation(s) across ${result.pageCount} page(s):`,
	];
	for (const violation of result.violations) {
		lines.push(
			`  ✗ [${violation.consumer}] ${violation.url} ("${violation.title}") — ${violation.detail}`,
		);
	}
	lines.push(
		"C-4 invariant broken: a page is not reachable from every consumer of the one build-time index (ADR-0058).",
	);
	return lines;
}
