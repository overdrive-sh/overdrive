import { readFileSync } from "node:fs";
import { resolve } from "node:path";

// Post-build verification that the one-index gate (ADR-0058) actually RAN and
// PASSED during `next build`. The real assertion lives in the force-static
// `app/one-index-check/route.ts`, which Next prerenders during the build; a
// violation throws there and fails `next build` outright. This verifier is the
// belt-and-braces guard against the gate SILENTLY VANISHING — e.g. the route
// segment getting renamed to a `_`-prefixed (private, un-routed) path again, or
// otherwise being dropped from the build so no assertion runs at all. It reads
// the prerendered body Next emits at `.next/server/app/one-index-check.body`
// and confirms it carries the PASSED summary. If the artifact is missing
// (gate didn't build) or doesn't say PASSED, this exits non-zero.
const BODY_PATH = resolve(
	process.cwd(),
	".next/server/app/one-index-check.body",
);

function main(): void {
	let body: string;
	try {
		body = readFileSync(BODY_PATH, "utf8");
	} catch {
		console.error(
			`✗ one-index gate verification FAILED — prerendered gate artifact not found at ${BODY_PATH}.\n` +
				"  The `app/one-index-check` force-static route did not prerender during `next build`,\n" +
				"  which means the C-4 assertion (ADR-0058) did NOT run. Re-run `next build` and ensure\n" +
				"  the route segment is routable (NOT `_`-prefixed).",
		);
		process.exit(1);
	}

	if (!body.includes("PASSED")) {
		console.error(
			`✗ one-index gate verification FAILED — gate artifact present but did not report PASSED:\n${body}`,
		);
		process.exit(1);
	}

	console.log(
		`✓ one-index gate verified — assertion ran during build and PASSED:\n  ${body.trim()}`,
	);
}

main();
