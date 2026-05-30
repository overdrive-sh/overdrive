import { server } from "@/lib/search";

// Node runtime, NEVER `runtime = 'edge'` (research § Decision C-2): the
// in-Worker Orama index is initialized in `lib/search.ts` over the build-time
// `source` and bounded by the 128 MB isolate ceiling — fine for the current
// corpus. OpenNext manages the Worker runtime.
export const runtime = "nodejs";

// Browser search transport: re-export the HTTP handler from the ONE-index seam.
// The programmatic `server.search(...)` over the SAME index is what slice 05's
// MCP `search_docs` consumes — see `lib/search.ts` for the seam contract.
export const { GET } = server;
