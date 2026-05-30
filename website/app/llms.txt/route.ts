import { llms } from "fumadocs-core/source";
import { source } from "@/lib/source";

// Node runtime, never edge (research § C-2): reads the build-time `source`
// index. OpenNext manages the Worker runtime.
export const runtime = "nodejs";

// `/llms.txt` — the corpus INDEX (third consumer of the ONE build-time `source`
// index after nav and search; DISCUSS C-4). `llms(source).index()` renders a
// markdown index of every doc URL in the same `source` the search seam and the
// `.md`/llms-full exports read. A page missing here is caught by
// `scripts/assert-one-index.ts` (consumer #2 check, ADR-0058).
export function GET(): Response {
	const index = llms(source).index();
	return new Response(index, {
		headers: { "Content-Type": "text/plain; charset=utf-8" },
	});
}
