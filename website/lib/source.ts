import { docs } from "@/.source/server";
import { loader } from "fumadocs-core/source";

// THE one build-time index (DISCUSS C-4). Every surface that searches or
// exports docs — browser search, MCP, llms.txt, blog (later slices) — is a
// consumer of this `source`, never a re-builder.
export const source = loader({
	baseUrl: "/docs",
	source: docs.toFumadocsSource(),
});
