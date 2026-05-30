import { createFromSource } from "fumadocs-core/search/server";
import { source } from "@/lib/source";

// ── The ONE-index search seam (ADR-0057, supersedes the slice-03 brief's
// inline `export const { GET } = createFromSource(source)` shape) ──
//
// `createFromSource` builds an in-Worker Orama index ONCE over the shared
// build-time `source` (DISCUSS C-4 — the same index slice 02's nav, slice 04's
// llms export, and slice 05's MCP all consume; never a second index). It
// returns a `SearchAPI` that exposes BOTH transports over that one index:
//
//   • `server.GET(request)`        — HTTP route handler. `app/api/search/route.ts`
//                                    re-exports this for the browser Cmd+K dialog.
//   • `server.search(query, opts)` — programmatic `(query) => Promise<SortedResult[]>`.
//                                    Slice 05's MCP `search_docs` tool imports
//                                    `server` from this module and calls
//                                    `server.search(query)` directly — same index,
//                                    no second `createFromSource` call site.
//   • `server.export` / `staticGET` — extras (static-index export), unused for now.
//
// Seam contract for slice 05: import { server } from "@/lib/search" and call
// `await server.search(userQuery, { limit })`. Do NOT call `createFromSource`
// again anywhere — that would build a divergent second index and break the
// ONE-index invariant. This module is the single initialization site.
export const server = createFromSource(source);
