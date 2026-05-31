import {
	checkOneIndex,
	formatOneIndexResult,
} from "@/scripts/assert-one-index";

// Node runtime, never edge (research § C-2): reads the build-time `source`
// index + the in-Worker Orama index.
export const runtime = "nodejs";

// Force-static: Next PRERENDERS this route during `next build`. That is the
// trick that makes the one-index assertion a build-time gate (ADR-0058) — the
// check runs inside the Next/Turbopack context where the fumadocs-mdx loader
// has resolved every page's processed markdown + structured data (which a bare
// `bun run` of the script cannot do). A violation `throw`s here, which fails
// `next build` with the structured message — exactly "wired into the build
// pipeline so a failure fails the build."
//
// NOTE: the route segment must NOT start with `_` — Next 16 treats `_`-prefixed
// folders as private and excludes them from routing, which silently dropped an
// earlier `__assert-one-index` variant from the build (and with it the gate).
// `one-index-check` is a routable, prerendered segment.
export const dynamic = "force-static";

export async function GET(): Promise<Response> {
	const result = await checkOneIndex();
	const lines = formatOneIndexResult(result);

	if (result.violations.length > 0) {
		// Logged so the failure is visible in the build worker output, then
		// thrown so `next build`'s static-generation step exits non-zero.
		for (const line of lines) console.error(line);
		throw new Error(lines.join("\n"));
	}

	for (const line of lines) console.log(line);
	return new Response(`${lines.join("\n")}\n`, {
		headers: { "Content-Type": "text/plain; charset=utf-8" },
	});
}
