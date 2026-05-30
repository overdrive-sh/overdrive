# Research: Fumadocs — Custom MCP Server, Search Integration, and Cloudflare Workers (Next.js + OpenNext)

**Date**: 2026-05-30 (updated 2026-05-30: closed gaps 1-4 with concrete code/config; added §4 blog support) | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High | **Sources**: 24

> **2026-05-30 — DECISION UPDATE (supersedes the original recommendation).** The framework
> decision is resolved: **Next.js (App Router, RSC) + `@opennextjs/cloudflare` (OpenNext) on
> Cloudflare Workers.** The original Executive Summary and § "Recommended Setup for overdrive.sh"
> recommended **TanStack Start** — that recommendation is **superseded**. The TanStack Start and
> React Router findings throughout §3 are **retained as rejected-alternative analysis** (the
> research is sound; the conclusion changed). Read the new **§ Decision** immediately below first;
> everything from the original "## Executive Summary" onward is preserved as the evidence trail and
> the rejected-alternative record. Rationale, evidence (four maintainer discussions), and the
> OpenNext-specific deltas are in § Decision.

---

## Decision (2026-05-30): Next.js + OpenNext on Cloudflare Workers

**Decision**: Build the overdrive.sh docs/blog/MCP site on **Next.js (App Router + RSC)**, deployed
to **Cloudflare Workers via `@opennextjs/cloudflare`**. Reject TanStack Start and React Router for
this site.

**One-line why**: Next.js is Fumadocs' reference target (RSC-native, fewest bug reports, the only
framework where *every* Fumadocs feature works), and OpenNext is the official, Cloudflare-partnered
path to run it on Workers — removing both the feature gate and the seven-month deployment-friction
trail that the Vite-framework paths carry.

### Why — the two decisive findings the original research under-weighted

**1. There is a hard *feature* gate on non-RSC frameworks, not just friction.** `fumadocs-openapi`
(the interactive API playground) is **RSC-native and renders blank on TanStack Start**; the
maintainer's guidance is to *"wait until they support RSC."* This gates the *class* of RSC-native
Fumadocs features away from TanStack Start **and** React Router (neither is RSC). For an
API-driven platform like overdrive.sh, the OpenAPI playground is plausibly wanted; on Next.js it is
available, on the Vite paths it is not. **Evidence**: [Discussion #3153 (2026-03-26)](https://github.com/fuma-nama/fumadocs/discussions/3153) — *"will be better to wait until they support RSC, Fumadocs OpenAPI is RSC native by design."*

**2. The "TanStack Start is early" caveat is corroborated over seven months by an expert user, not a
stale one-off.** A decades-experienced docs-site builder (jkinley) hit a continuous trail of
TanStack-specific friction, and the maintainer repeatedly routes TanStack×Nitro×deploy-preset bugs
to TanStack's own Discord because *"Fumadocs is simply a layer over the original React framework."*
You would own debugging at an intersection no single party supports. **Evidence trail**:
- [Discussion #2442 (2025-10 → 2026-05)](https://github.com/fuma-nama/fumadocs/discussions/2442) — fully-static-on-CDN needs the **experimental** `@tanstack/start-static-server-functions` plugin + manual `404.html` shell hacks; final community finding (2026-03-02): *"it still invokes a worker request for non-existent routes... Would go full SPA Cloudflare pages if you want to 100% avoid worker requests"*; an unanswered "anyone got SSG working?" sits at the tail dated **2026-05-16**.
- [Discussion #2880 (2026-01)](https://github.com/fuma-nama/fumadocs/discussions/2880) — a 16.0→16.4 upgrade broke the page tree (`useFumadocsLoader()` hard-expects a property named `pageTree`; the generated TanStack template used `tree`); maintainer: *"I agree it's not an intuitive behaviour."*
- [Discussion #2969 (2026-02)](https://github.com/fuma-nama/fumadocs/discussions/2969) — base-path / PR-preview-SHA URLs on S3 under SPA mode; deflected to TanStack Discord.
- [Discussion #2442 (2025-10-21)](https://github.com/fuma-nama/fumadocs/discussions/2442) — the maintainer's maturity ranking and RSC rationale, verbatim: *"Next.js has been the most mature solution with RSC support (which is pretty ideal as we're RSC-native), I also got fewer bug reports from it… Tanstack Start is too early for production, details like SPA isn't clear. React Router works good… using Next.js wouldn't be overkill if I want the most stable RSC support."* The maintainer also notes Cloudflare specifically *"has to consider Node.js compat"* — which is exactly what OpenNext provides.

**Feature vs. deployment — the distinction that resolves "is everything supported?"** Almost all
Fumadocs capabilities are framework-agnostic (Web-standard APIs) and work on every supported
framework — core docs (`DocsLayout`, page tree, MDX, TOC, tabs/accordions/steps), search (Orama
`/api/search`), `llms.txt`/`llms-full.txt`/per-page `.md`, and a custom MCP server (§1, §2 of this
doc remain valid as written). The **only** named *feature* gap is the **RSC-native** class,
headlined by `fumadocs-openapi`. Everything painful in the four discussions above was
*deployment/build* friction (static export, base paths, an upgrade rename, worker-request-on-404),
not missing features. Next.js eliminates the feature gate **and** sidesteps that deployment friction
class (no `@cloudflare/vite-plugin` SSR module-duplication bug — see Finding "duplicate React" in the
React Router note below — because OpenNext wraps Next's own build rather than using the Vite SSR
environment).

### OpenNext-specific constraints (the deltas from the Vite path in §3)

These **replace** §3.2 (Vite/MDX triad) and §3.3 (`@cloudflare/vite-plugin`) for the chosen path:

- **Scaffold**: `npm create cloudflare@latest -- <name> --framework=next --platform=workers` — wires
  OpenNext, Wrangler, and `nodejs_compat`. [OpenNext — Cloudflare](https://opennext.js.org/cloudflare); [Cloudflare — Next.js framework guide](https://developers.cloudflare.com/workers/framework-guides/web-apps/nextjs/).
- **Node runtime everywhere — never `export const runtime = 'edge'`.** OpenNext runs Next in the
  Workers **Node** runtime and does **not** support the edge-runtime directive (the adapter manages
  the runtime itself). This applies to every Fumadocs route handler you add (`/api/search`,
  `llms.txt`, `/mcp`) — leave them on the default Node runtime. `cookies()` from `next/headers` is
  Node-only, which is fine on this path.
- **Next 15 or 16.** Next.js 14 support was dropped Q1 2026; 16 and the latest 15 minors are
  supported. App Router, RSC, Server Actions, ISR/SSR, middleware, and PPR are all supported.
- **MDX uses the Next plugin, not the Vite triad.** Replace §3.2.2's `source.config.ts` +
  `fumadocs-mdx/vite` + `collections/server` with the **Next path**: `createMDX()` from
  `fumadocs-mdx/next` wrapping `next.config`, the docs catch-all at `app/docs/[[...slug]]/page.tsx`,
  and `lib/source.ts` over the Next-emitted source. The framework-agnostic `loader()` /
  `source.getPages()` / `getLLMText()` surface (§1.2, §1.3) is **identical** — only the build plugin
  and route file-shape change.
- **ISR/on-demand revalidation needs a cache binding (R2 recommended; KV/D1 also work).** A docs
  site is overwhelmingly build-time/SSG — SSG pages serve as static assets and need no cache binding.
  Only wire the R2 incremental cache if you actually use `revalidate`.
- **`next/image` optimization needs a custom loader or Cloudflare Images.** For docs, an `unoptimized`
  config or a simple loader is usually sufficient.
- **Bundle ceiling**: Worker compressed-bundle limit is 3 MiB Free / 10 MiB Paid — ample for a docs
  site, but the same ceiling the in-Worker Orama index shares (Finding 3.4.4 still governs).

### What carries over unchanged from the original research

- **§1 (Custom MCP server)** — fully valid. Build the stateless MCP server reusing the build-time
  `source` index; expose `search_docs` + `get_doc`→`getLLMText`. On this path the cleanest shape is a
  **Next route handler** (`app/mcp/route.ts`) on the Node runtime, reusing the same index that feeds
  `/api/search` and `llms-full.txt` (one index, three consumers — Finding 1.3.1). With OpenNext,
  bolting an arbitrary non-Next route onto the same Worker is awkward, so the route handler (or a
  separate dedicated Worker) is preferred over the §3.4.3 "same-Worker fetch route" framing.
- **§2 (Search)** — the three architectures (in-Worker Orama → static export → external
  Algolia/Orama Cloud) and the edge-friendliness ordering all still hold. On Next.js the in-Worker
  Orama handler is `app/api/search/route.ts` with `export const { GET } = createFromSource(source)`
  (the documented Next default — *simpler* than the TanStack `createFileRoute().server.handlers`
  adapter seam flagged in Finding 2.1.2, which no longer applies). The 128 MB isolate ceiling
  (Finding 3.4.4) still bounds in-Worker search.
- **§3.4.1 (no runtime `fs`)** — still load-bearing and still satisfied: MDX is compiled into the
  bundle at build time by `fumadocs-mdx` regardless of Next-vs-Vite, so the Worker never reads
  `content/` at request time.
- **§4 (Blog)** — fully valid; the blog is a second `defineCollections({ type: 'doc' })` collection
  feeding the same index. On Next.js the blog's list/post routes are the **documented** Next
  components (`app/(home)/blog/page.tsx`, `app/(home)/blog/[slug]/page.tsx`) — the §4.4(c) "TanStack
  port is inference" caveat **disappears**; these are copy-paste from the guide, not a port.

### Rejected alternatives (analysis retained in §3 below)

- **TanStack Start** — supported and documented, but the highest-friction supported path (seven-month
  evidence trail above), and RSC-gated out of `fumadocs-openapi`. Viable only if (a) you never want
  RSC-native features and (b) you go **fully static** (SPA + prerender → Cloudflare Pages / Workers
  static-assets, `not_found_handling: "404-page"` to kill the worker-request-on-404). See §3.1–3.4.
- **React Router v7** — maintainer-ranked above TanStack Start ("works good"), but **also non-RSC**
  (same OpenAPI gate) and, on Workers via `@cloudflare/vite-plugin`, **SSR-only** (no SPA/prerender)
  with a known `workerd` SSR duplicate-React "Invalid hook call" bug fixable via
  `ssr.noExternal: [/fumadocs.*/, "react", "react-dom"]` + `resolve.dedupe`
  ([workers-sdk #11825](https://github.com/cloudflare/workers-sdk/issues/11825)). Rejected for the
  same RSC-gate reason as TanStack Start, with no maturity upside over Next.js.

## Executive Summary

> **Superseded by § Decision (2026-05-30).** This summary recommends TanStack Start; the resolved
> decision is **Next.js + OpenNext**. Retained verbatim as the original research record and the
> rejected-alternative rationale. The framework-agnostic findings (MCP §1, search §2, blog §4) remain
> valid on the chosen Next.js path; the TanStack/Vite-specific findings (§3) are rejected-alternative.

This research targets the *specific* overdrive.sh stack — Fumadocs + TanStack Start + Cloudflare Workers + a custom MCP server — not the common Fumadocs-on-Next.js-on-Vercel path. All three concerns are achievable on official, documented paths, but the stack is assembled from three official tutorials that each stop at their own boundary; there is **no single official "Fumadocs MCP server on TanStack Start on Workers" walkthrough**.

The central caveat is framework maturity: the Fumadocs maintainer (fuma-nama) explicitly calls TanStack Start "too early for production" and steers users to the **TanStack Start SPA template** (and ranks Next.js > React Router > TanStack Start by maturity). The SPA/prerender shape is also the most Cloudflare-Workers-friendly, so the maintainer's recommendation and the deployment constraint point the same direction. Cloudflare officially supports TanStack Start via the `@cloudflare/vite-plugin` (since 2025-10-24), so the leg is supported — just the least battle-tested.

On search and MCP the picture is clean: Fumadocs' framework-agnostic core exposes the docs corpus through one `source` Loader (`source.getPages()`, `getLLMText(page)`, `structuredData`) that simultaneously feeds `/api/search` (Orama), `llms-full.txt`, and a custom MCP server. The build-time-bundling model means the Worker never needs filesystem access at runtime (which matters: Workers `node:fs` exists but is an ephemeral virtual FS that can't read your `content/` dir). The recommended MCP path on Workers is Cloudflare's stateless `createMcpHandler()` exposing `/mcp` over Streamable HTTP, with tool-call logging to Analytics Engine/D1 for the analytics loop.

**Key bullets:**
- **No first-party `fumadocs-mcp` package.** Fumadocs' agent story is content-export (llms.txt / llms-full.txt / per-page `.md` via `getLLMText`); you build the MCP server yourself — which matches the "own the runtime + log tool calls" goal.
- **Fumadocs is framework-agnostic since the v14→v16 Content→Core→UI split**; TanStack Start is a first-class target (`fumadocs-ui/provider/tanstack`, docs route `routes/docs/$.tsx`, search route `src/routes/api/search.ts`). **Pin v16** for the Vite path. The Vite MDX triad is concrete: `source.config.ts` (`defineDocs` from `fumadocs-mdx/config`) + `vite.config.ts` (`mdx()` from `fumadocs-mdx/vite`) + `lib/source.ts` (`loader()` over `docs.toFumadocsSource()`) — Finding 3.2.2.
- **BLOCKER (soft):** TanStack Start is maintainer-flagged immature; use the SPA template. It is *supported*, not *unsupported* — Cloudflare's official Vite-plugin guide covers it.
- **Cloudflare Workers needs `nodejs_compat` + `main: "@tanstack/react-start/server-entry"`**; static prerender needs TanStack Start v1.138.0+. No runtime fs for content — rely on build-time MDX compilation into the bundle.
- **Three search architectures**, edge-friendliness ascending: in-Worker Orama (`createFromSource`) → static export (browser-built index) → external Algolia/Orama Cloud (build-time `sync()`). Start in-Worker, migrate external as the corpus grows.
- **MCP on Workers = stateless `createMcpHandler()` at `/mcp`** (Streamable HTTP), reusing the same build-time Orama index for `search_docs` + `get_doc(url)→getLLMText`. One index, three consumers.
- **Use `@modelcontextprotocol/sdk`** (single package, subpath imports) if going raw; prefer the Cloudflare Agents SDK on Workers.
- **Blog support is a BYO-UI pattern, not a layout** (§4). Fumadocs ships the docs `<DocsLayout>` but **no `<BlogLayout>`** — a blog is a *second* `defineCollections({ type: 'doc', dir: 'content/blog' })` collection (flat posts, no catch-all) feeding the **same build-time index**, so the MCP server + Orama search index it for free and it's Workers-compatible on the same no-runtime-`fs` terms; you hand-roll the list/post pages, frontmatter schema, and RSS/OG yourself, and the TanStack Start port of the guide's Next.js route components is inference (Findings 4.1–4.4).

## Research Methodology
**Search Strategy**: Official Fumadocs docs (fumadocs.dev) + GitHub repo (fuma-nama/fumadocs) as primary; Cloudflare, TanStack, Orama, MCP official docs as secondary authorities.
**Source Selection**: official_publication tier for subject-matter docs; cross-referenced against repo source where possible.
**Quality Standards**: Target 3 sources/claim (min 1 authoritative). Version-pinned where APIs move fast.

---

## 1. Custom MCP Server (Build-Time Doc Indexing + Agent Exposure)

### 1.1 What Fumadocs ships out of the box (LLM/MCP-facing)

**Finding 1.1.1 — Fumadocs ships NO first-party MCP server / `fumadocs-mcp` package.**
**Evidence**: The official AI & LLMs integration page documents llms.txt, llms-full.txt, per-page `.md` export, "Page Actions" and "Ask AI" (OpenRouter/Inkeep) — but contains no first-party MCP server or `fumadocs-mcp` package. (WebFetch summary: "No first-party MCP server or `fumadocs-mcp` package is mentioned in this documentation.")
**Source**: [Fumadocs — AI & LLMs](https://www.fumadocs.dev/docs/integrations/llms) — Accessed 2026-05-30
**Confidence**: Medium (single official page; corroborated below by the fact that the documented agent path is llms.txt + LangChain `mcpdoc`, not a Fumadocs MCP package)
**Analysis**: Fumadocs' "agent story" is **content-export-first**: it makes the corpus machine-readable (llms.txt / llms-full.txt / `.md`), and leaves the MCP server to be built by the site owner (or to a generic llms.txt-consuming MCP like LangChain `mcpdoc`). For overdrive.sh's goal of *owning* the MCP runtime and logging tool calls, this is actually the expected shape — you build the MCP server; Fumadocs gives you the indexable content surface.

### 1.2 llms.txt / llms-full.txt / markdown-export conventions

**Finding 1.2.1 — `getLLMText()` + `getText('processed')` is the canonical content-extraction primitive.**
**Evidence**:
```ts
// requires includeProcessedMarkdown: true in source config
export async function getLLMText(page: (typeof source)['$inferPage']) {
  const processed = await page.data.getText('processed');
  return `# ${page.data.title} (${page.url})\n\n${processed}`;
}
```
`llms.txt` is the index via `llms(source).index()`; `llms-full.txt` maps all pages through `getLLMText()` and joins. Per-page `.md` access by appending `.md` to a doc URL. Accept-header negotiation via `isMarkdownPreferred()`.
**Source**: [Fumadocs — AI & LLMs](https://www.fumadocs.dev/docs/integrations/llms) — Accessed 2026-05-30
**Confidence**: High (official page, exact API)
**Analysis**: This extraction primitive (`source.getPages()` → `getLLMText`) is framework-agnostic — it operates on the Fumadocs `source` Loader object, NOT on Next.js. The page explicitly notes "Other frameworks (React Router, Tanstack Start, Waku) follow similar patterns with framework-specific routing syntax." This is the build-time/request-time hook a custom MCP server reuses.

### 1.3 Building a custom MCP server over the Fumadocs content source

**Finding 1.3.1 — A custom MCP server reuses the Fumadocs `source` Loader: index `source.getPages()` at build time, expose `search_docs` / `get_doc` tools that return `getLLMText(page)` content.**
**Evidence (synthesis of two primary sources)**: Fumadocs exposes the corpus via the `source` object (`source.getPages()`, `page.data.getText('processed')`, `page.url`, `page.data.structuredData`) — the same primitives `llms-full.txt` and `createSearchAPI('advanced')` consume. The MCP TypeScript SDK registers tools on an `McpServer` with zod input schemas returning `{ content: [{ type: 'text', text }] }`.
**Sources**: [Fumadocs — AI & LLMs](https://www.fumadocs.dev/docs/integrations/llms); [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama) (`source.getPages().map(... structuredData)`); [MCP TypeScript SDK — server.md](https://github.com/modelcontextprotocol/typescript-sdk/blob/main/docs/server.md) — Accessed 2026-05-30
**Confidence**: High (the building blocks are each individually documented in official sources; the *composition* is the analysis)
**Analysis — recommended shape**: Build a tiny Orama index from `source.getPages()` (reuse `createSearchAPI('advanced')`'s index shape — title/description/url/structuredData), then:
- `search_docs(query)` → run the in-memory Orama query, return ranked `{ title, url, excerpt }`.
- `get_doc(url)` → look up the page, return `getLLMText(page)` (title + processed markdown).
This keeps ONE indexing pass shared between `/api/search`, `llms-full.txt`, and the MCP server. For the analytics loop, wrap each tool handler with a logging call (write the tool name + args + result-size to a Workers Analytics Engine binding or D1 — see §3.4) before returning.

**Finding 1.3.2 — The documented "generic" agent path is llms.txt + LangChain `mcpdoc`, not a Fumadocs MCP package.**
**Evidence**: LangChain's `mcpdoc` MCP server takes a user-defined list of llms.txt files and exposes a `fetch_docs` tool to read URLs within them — the standard way to put any llms.txt site into the MCP ecosystem.
**Source**: [langchain-ai/mcpdoc](https://github.com/langchain-ai/mcpdoc) — Accessed 2026-05-30
**Confidence**: Medium (single GitHub source; relevant as the "off-the-shelf alternative" baseline)
**Analysis**: `mcpdoc` is a viable zero-build fallback (point it at `https://overdrive.sh/llms.txt`), but it does NOT give you the tool-call analytics loop or custom `search_docs` ranking — it just fetches URLs. For overdrive.sh's stated goal (own the runtime, log tool calls), the custom server in 1.3.1 is the right call; `mcpdoc` is the "what you'd get for free" comparison.

### 1.4 Transport: stdio vs remote (HTTP/SSE streamable) + deployment

**Finding 1.4.1 — Two transports: stdio (local child process) vs Streamable HTTP (remote, single endpoint, optional SSE streaming). Streamable HTTP is the current spec standard.**
**Evidence**: MCP spec/SDK: stdio = server reads JSON-RPC from stdin/writes stdout, spawned as a child process (Claude Desktop, CLI). Streamable HTTP = independent network process, HTTP POST/GET on a single endpoint, optional SSE to stream multiple server messages; `sessionIdGenerator` set = stateful, `undefined` = stateless.
**Sources**: [MCP — Transports spec (2025-03-26)](https://modelcontextprotocol.io/specification/2025-03-26/basic/transports); [MCP TypeScript SDK — server.md](https://github.com/modelcontextprotocol/typescript-sdk/blob/main/docs/server.md) — Accessed 2026-05-30
**Confidence**: High (official spec + official SDK)

**Finding 1.4.2 — On Cloudflare Workers, the native path is the Agents SDK: `createMcpHandler()` (stateless) or `McpAgent` (Durable Object per session), exposing a `/mcp` endpoint over Streamable HTTP.**
**Evidence**: Cloudflare Agents SDK offers (1) `createMcpHandler()` — stateless, fastest, no Durable Objects; (2) `McpAgent` — DO per session, SSE + Streamable HTTP; (3) raw `WebStandardStreamableHTTPServerTransport` via `@modelcontextprotocol/sdk` for full control. Server exposes `/mcp`; tools defined with `server.tool()` + zod. Deploy `npx wrangler deploy`. Clients connect via MCP Inspector, the `mcp-remote` proxy (bridges Claude Desktop/Cursor that don't yet speak remote), or AI Playground. OAuth 2.0 supported for auth.
**Source**: [Cloudflare Agents — Build a Remote MCP server](https://developers.cloudflare.com/agents/guides/remote-mcp-server/) — Accessed 2026-05-30
**Verification**: Cloudflare blog corroborates streamable HTTP support landing for MCP servers. [Cloudflare Blog — Streamable HTTP + Python for MCP](https://blog.cloudflare.com/streamable-http-mcp-servers-python/)
**Confidence**: High (official Cloudflare docs + blog)
**Analysis**: For overdrive.sh: a docs-query MCP server is **stateless** (no per-session state needed), so `createMcpHandler()` is the simplest fit and avoids a Durable Object dependency. It can live in the SAME Worker as the docs site (route `/mcp` in the fetch handler) or a separate Worker. Stateless also sidesteps DO billing/complexity. Use `McpAgent` only if you later need per-session state (e.g. conversation-scoped context).

---

## 2. Search Integration

### 2.1 Built-in Orama search (`createFromSource`, `/api/search`)

**Finding 2.1.1 — Orama is the default; `createFromSource(source, ...)` from `fumadocs-core/search/server` is the canonical server handler, and the route path is framework-specific.**
**Evidence**:
```ts
import { source } from '@/lib/source';
import { createFromSource } from 'fumadocs-core/search/server';
export const { GET } = createFromSource(source, { language: 'english' });
```
Route handler location by framework:
- Next.js: `app/api/search/route.ts`
- React Router: `app/routes/search.ts`
- **TanStack Start: `src/routes/api/search.ts`**
- Waku: `src/pages/_api/api/search.ts`

Alternative `createSearchAPI('advanced', { indexes: source.getPages().map(...) })` for manual index construction with `structuredData`.
**Source**: [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama) — Accessed 2026-05-30
**Confidence**: High (official page, exact API, explicit per-framework route table)
**Analysis**: The same per-framework route table appears in the TanStack Start install page (search at `src/routes/api/search.ts`), cross-confirming the framework-agnostic split. Orama is an **in-process JS search engine** — the index lives in memory in the Worker/route handler; this matters for the Workers runtime (§3.4).

**Finding 2.1.2 — The TanStack Start search route diverges from the Next.js `export const { GET }` shape: Fumadocs assigns `createFromSource(source)` to a `server` const, and the route must adapt it to TanStack's `createFileRoute(...).server.handlers.GET` (GAP 2 — CLOSED, with documented seam flagged).**
**Evidence (Next.js documented default):**
```ts
// Next.js: app/api/search/route.ts — createFromSource returns a { GET } handler object
import { source } from '@/lib/source';
import { createFromSource } from 'fumadocs-core/search/server';
export const { GET } = createFromSource(source, { language: 'english' });
```
**Evidence (TanStack Start — as documented on the Fumadocs install page):**
```ts
// src/routes/api/search.ts (Fumadocs TanStack Start install page)
import { source } from '@/lib/source';
import { createFromSource } from 'fumadocs-core/search/server';

const server = createFromSource(source, {
  language: 'english',
});
```
The Fumadocs TanStack Start page assigns the result to `const server` (NOT `export const { GET }`). `createFromSource` is framework-agnostic: it returns Web-standard handlers (`server.GET` is a `(request: Request) => Response`-shaped function), which is what TanStack Start's server-route layer expects.
**Source (Fumadocs)**: [Fumadocs — TanStack Start install](https://www.fumadocs.dev/docs/manual-installation/tanstack-start); [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama) — Accessed 2026-05-30
**Source (TanStack server-route export shape)**: [TanStack Start — Server Routes](https://tanstack.com/start/latest/docs/framework/react/guide/server-routes) — Accessed 2026-05-30
**Confidence**: Medium-High (both halves are official; the *wiring of the two* — handing `createFromSource`'s `GET` into TanStack's `server.handlers.GET` — is inferred, see below).
**Analysis — the documented seam (inference vs documented)**: TanStack Start's own server-route API (current `@tanstack/react-router` Start, server-routes guide) defines HTTP handlers as:
```ts
import { createFileRoute } from '@tanstack/react-router';
export const Route = createFileRoute('/api/search')({
  server: {
    handlers: {
      GET: async ({ request }) => server.GET(request), // server = createFromSource(source, ...)
    },
  },
});
```
The Fumadocs install page stops at `const server = createFromSource(...)` and does **not** show the `createFileRoute(...).server.handlers.GET` binding. The binding above is **inferred** by composing the Fumadocs Web-standard handler (`server.GET(request: Request): Response`) with TanStack Start's documented `server.handlers.GET: ({ request }) => Response` signature — both are Web-`Request`/`Response`-shaped, so the adapter is a one-line delegation. This is the one genuine TanStack-Start-specific divergence from the Next.js default (where `export const { GET }` is consumed directly by the App Router). **Caveat**: older TanStack Start examples used a now-removed `createServerFileRoute(...).methods({ GET })` API; the current shape is `createFileRoute(...).server.handlers`. Re-verify against the installed `@tanstack/react-router` Start version, since this API moved during the Start beta.

### 2.2 Static export search

**Finding 2.2.1 — Static mode uses `staticGET` server-side + `type: 'static'` client; pre-renders the index as JSON the client downloads.**
**Evidence**: "For static deployments, use `staticGET` instead of the dynamic `GET` handler. This pre-renders search indexes as JSON files." Client: `useDocsSearch({ type: 'static', from: '/api/search', initOrama: (locale) => create({ schema: { _: 'string' } }) })`. **Warning** in the docs: "Large documentation sites should prefer cloud solutions like Orama Cloud or Algolia, as static search requires clients downloading exported indexes."
**Source**: [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama) — Accessed 2026-05-30
**Confidence**: High (official)
**Analysis**: Static mode is the most edge-friendly path (no runtime search compute — the client builds the Orama index from a downloaded JSON), but trades client bandwidth/CPU. For a focused docs corpus it is viable; the docs themselves steer large sites to Orama Cloud / Algolia.

### 2.3 Third-party: Algolia, Orama Cloud

**Finding 2.3.1 — Algolia integration is build-time-sync based: export `static.json` via `exportSearchIndexes()`, then `sync()` from `fumadocs-core/search/algolia` pushes records; client uses `useDocsSearch({ type: 'algolia', ... })`.**
**Evidence**:
```ts
// build script
import { algoliasearch } from 'algoliasearch';
import { sync, DocumentRecord } from 'fumadocs-core/search/algolia';
// reads generated static.json, syncs records after build
```
```ts
// client
import { liteClient } from 'algoliasearch/lite';
import { useDocsSearch } from 'fumadocs-core/search/client';
const { search, setSearch, query } = useDocsSearch({ type: 'algolia', indexName: 'document', client });
```
"It creates a record for each paragraph in your document" (searchable attrs: title, section, content).
**Source**: [Fumadocs — Algolia Search](https://www.fumadocs.dev/docs/headless/search/algolia) — Accessed 2026-05-30
**Confidence**: High (official, exact code)
**Analysis — best fit for Workers + edge**: Algolia (or Orama Cloud) offloads the index to an external service queried by the client; the Worker does **no search compute at runtime** and ships **no index to the client**. This is the cleanest edge model for a growing corpus — it sidesteps both the static-mode client-download cost and the in-memory-index-per-isolate cost of `createFromSource` running inside the Worker. Trade-off: an external dependency + API keys + a build-time sync step.
**Finding 2.3.2 — Orama Cloud integration: build-time `sync()` from `fumadocs-core/search/orama-cloud` pushes the index; client uses `useDocsSearch({ type: 'orama-cloud', client })` with the `@orama/core` `OramaCloud` client (GAP 2/3 — CLOSED).**
**Evidence (build-time index push + client wiring, from the Fumadocs Orama Cloud headless page):**
```ts
// build script — push the docs index to Orama Cloud after build
import { sync, type OramaDocument } from 'fumadocs-core/search/orama-cloud';
// sync(...) reads the generated docs records and pushes them to the Orama Cloud datasource.
// REST API data-source env: ORAMA_PRIVATE_API_KEY (private, build-time),
//   plus the public NEXT_PUBLIC_ORAMA_DATASOURCE_ID / _PROJECT_ID / _API_KEY for the client.
```
```ts
// client — custom SearchDialog backed by Orama Cloud
import { OramaCloud } from '@orama/core';            // Orama Cloud client SDK
import { useDocsSearch } from 'fumadocs-core/search/client';

const client = new OramaCloud({ /* projectId / apiKey from public env */ });
const { search, setSearch, query } = useDocsSearch({ type: 'orama-cloud', client });
```
Two data-source modes are documented: a **REST API** datasource (the `sync()`-pushed index above, using the four `ORAMA_*` env vars) and a **Web Crawler** datasource (`index: 'crawler'` with a read-only API key — Orama crawls the deployed site, no build-time `sync()`).
**Source**: [Fumadocs — Orama Cloud Search (Headless)](https://www.fumadocs.dev/docs/headless/search/orama-cloud) — Accessed 2026-05-30
**Verification**: The `@orama/core` `OramaCloud` client package is corroborated by Orama's own docs. [Orama Docs — Orama Cloud](https://docs.orama.com/cloud); the `useDocsSearch` `type: 'orama-cloud'` mode cross-confirms the client-mode table in Finding 2.4.2. [Fumadocs — UI Search](https://www.fumadocs.dev/docs/ui/search)
**Confidence**: Medium-High (exact imports + env vars from the dedicated Fumadocs Orama Cloud page; the `OramaCloud` constructor signature was paraphrased, not fully quoted — re-verify the exact `new OramaCloud({...})` options against docs.orama.com before pinning).
**Note (package naming)**: The Fumadocs page references `@orama/core` for the `OramaCloud` client; some older Orama Cloud docs use `@oramacloud/client`. Both names appear in the ecosystem — `@orama/core` is the name the current Fumadocs Orama Cloud page cites; confirm the installed package name against the version of Orama Cloud's SDK in use. **The build-time `sync` import is unambiguous: `fumadocs-core/search/orama-cloud`.**
**Analysis — best fit for Workers + edge**: Orama Cloud (like Algolia) offloads the index to an external service queried by the client; the Worker does **no search compute at runtime** and ships **no index to the client**. This is the cleanest edge model for a growing corpus — it sidesteps both the static-mode client-download cost and the in-memory-index-per-isolate cost of `createFromSource` running inside the Worker (the ceiling quantified in Finding 3.4.4). Trade-off: an external dependency + API keys + a build-time `sync()` step (or the crawler datasource, which removes even the build step).

### 2.4 Client wiring (`<RootProvider>`, `SearchDialog`)
**Finding 2.4.1 — Search client modes: `fetch` (default, hits `/api/search`), `static` (downloads JSON), via `useDocsSearch` from `fumadocs-core/search/client`; UI is driven through `RootProvider`/`SearchDialog`.**
**Evidence**: `useDocsSearch({ type: 'fetch', api: '/api/search' })` (default); `type: 'static'` for static. "Headless Usage: Access `initAdvancedSearch` to integrate search beyond Fumadocs UI components."
**Source**: [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama) — Accessed 2026-05-30
**Confidence**: Medium (RootProvider search-config / SearchDialog override specifics resolved in Finding 2.4.2)

**Finding 2.4.2 — `<RootProvider search={{...}}>` config + custom `SearchDialog` swap (GAP 3 — CLOSED).**
**Evidence (the `search` prop on `RootProvider`, from the Fumadocs UI Search page):**
```tsx
// __root.tsx (TanStack: import { RootProvider } from 'fumadocs-ui/provider/tanstack')
import { RootProvider } from 'fumadocs-ui/provider/tanstack';

<RootProvider
  search={{
    enabled: true,                  // default true — toggle search entirely
    preload: true,                  // default true — preload the dialog before first open
    hotKey: [{ /* HotKey[] */ }],   // default Meta/Ctrl + K
    links: [/* SearchLink[] — shown when query is empty */],
    options: { /* Partial<DefaultSearchDialogProps> — e.g. { type: 'fetch', api: '/api/search' } */ },
    SearchDialog: MyCustomSearchDialog, // React.ComponentType<SharedProps> — swap the dialog wholesale
  }}
>
  {children}
</RootProvider>
```
The default (no `SearchDialog`) renders Fumadocs' built-in dialog, which uses the **fetch-based** client hitting `/api/search`. The default client is:
```tsx
import { useDocsSearch } from 'fumadocs-core/search/client';
const { search, setSearch, query } = useDocsSearch({ type: 'fetch', locale /* optional */ });
```
To supply a **custom** `SearchDialog` (e.g. for Orama Cloud or Algolia), pass a component to `search.SearchDialog`; it receives `SharedProps` (`open`, `onOpenChange`) and composes the dialog primitives from `fumadocs-ui/components/dialog/search` (`SearchDialog`, `SearchDialogContent`, `SearchDialogInput`, `SearchDialogList`, …), wiring its own `useDocsSearch({ type: 'orama-cloud' | 'algolia', client, ... })` hook.
**Source**: [Fumadocs — UI Search](https://www.fumadocs.dev/docs/ui/search) — Accessed 2026-05-30
**Verification**: `useDocsSearch` client modes (`fetch`/`static`/`algolia`/`orama-cloud`) cross-confirmed across the Orama, Algolia, and Orama Cloud headless pages. [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama); [Fumadocs — Algolia Search](https://www.fumadocs.dev/docs/headless/search/algolia)
**Confidence**: High (exact prop table + import paths from the dedicated UI Search page; cross-confirmed client modes)
**Analysis**: The `search` prop is the single integration seam — `enabled`/`preload`/`hotKey`/`links` tune the default dialog; `options` configures the default client (e.g. point `api` elsewhere); `SearchDialog` is the full escape hatch for an external provider. For overdrive.sh: keep the default fetch dialog for in-Worker Orama (§3.4 option 2); swap `SearchDialog` only when migrating to Orama Cloud / Algolia (§2.3).

---

## 3. Cloudflare Workers + TanStack Start

> **Rejected-alternative analysis (per § Decision, 2026-05-30).** This entire section documents the
> TanStack Start / `@cloudflare/vite-plugin` path, which was **not** chosen. For the selected
> Next.js + OpenNext path, the framework + deploy + MDX deltas live in § Decision → "OpenNext-specific
> constraints" (which replace §3.1–§3.3). §3.4 (no-runtime-`fs`, the three search architectures, the
> 128 MB Orama ceiling) is framework-agnostic and **still applies** on the Next.js path; only the
> route file-shapes and the "same-Worker fetch route" MCP framing change. Findings below are retained
> for the evidence trail and the rejected-alternative record.

### 3.1 Fumadocs on TanStack Start (framework-agnostic core)

**Finding 3.1.1 — Fumadocs runs on TanStack Start via `fumadocs-core` + `fumadocs-ui` (no Next.js plugin); the install path is officially documented.**
**Evidence**: Install `fumadocs-core` and `fumadocs-ui`. Provider: `import { RootProvider } from 'fumadocs-ui/provider/tanstack'` wrapped in `__root.tsx`. Docs route is a TanStack file-route catch-all `routes/docs/$.tsx` using `createServerFn` (server loader) + `DocsLayout` + `useMDXComponents()`. Search route `routes/api/search.ts`. Content in `content/docs/`.
**Source**: [Fumadocs — TanStack Start manual install](https://www.fumadocs.dev/docs/manual-installation/tanstack-start) — Accessed 2026-05-30
**Verification**: Framework-agnostic split corroborated by the per-framework route tables on both the LLMs page and the Orama search page (both enumerate TanStack Start as a first-class target). [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama)
**Confidence**: High (official install page + two cross-confirming official pages)

**Finding 3.1.2 — The maintainer (fuma-nama) explicitly flags TanStack Start as immature for production and steers toward SPA mode.**
**Evidence**: "Tanstack Start is too early for production, details like SPA isn't clear" (Oct 21). "Next.js has been the most mature solution with RSC support"; "React Router works good." Fumadocs added a **TanStack Start SPA template** (Oct 30). For Cloudflare Workers specifically the maintainer advised "use the SPA template and host the files statically" and to consult TanStack's community for deployment specifics.
**Source**: [Fumadocs GitHub Discussion #2442 — "Fumadocs + Tanstack Start fully static"](https://github.com/fuma-nama/fumadocs/discussions/2442) — Accessed 2026-05-30
**Confidence**: Medium-High (direct maintainer statements; single thread — a community/maintainer source, dated Oct 2025)
**Analysis**: This is the central caveat for overdrive.sh. Fumadocs-on-TanStack-Start *works* and is documented, but the maintainer's own posture is "early." The recommended robust path is the **TanStack Start SPA template** (prerender/static) rather than full SSR — which also happens to be the most Workers-friendly shape (§3.4).

### 3.2 Content source + MDX processing without the Next.js plugin

**Finding 3.2.1 — MDX is processed via the Fumadocs MDX *Vite* setup (`source.config.ts` + `lib/source.ts`), not the Next.js plugin.**
**Evidence**: The TanStack Start page directs: "Fumadocs MDX: follow the Vite setup guide" and references a pre-configured `lib/source.ts`. The docs route loads page data via a client-side loader "from your collections" defined in `source.config.ts`.
**Source**: [Fumadocs — TanStack Start manual install](https://www.fumadocs.dev/docs/manual-installation/tanstack-start) — Accessed 2026-05-30
**Confidence**: Medium (the page references the Vite setup guide rather than inlining it — the exact `source.config.ts`/Vite-plugin code needs the dedicated Vite/MDX page; resolved in Finding 3.2.2)
**Analysis**: Because MDX is compiled at **build time** by the Vite plugin into the bundle, the runtime does NOT need `fs` to read `.mdx` files — the compiled content is part of the JS bundle / generated source map. This is the property that makes the Workers runtime (no Node fs at request time) viable. The exact build-time bundling shape is confirmed in Finding 3.2.2.

**Finding 3.2.2 — The concrete `source.config.ts` + `fumadocs-mdx/vite` plugin + `lib/source.ts` triad (GAP 1 — CLOSED).**
**Evidence (three exact files, from the dedicated Fumadocs MDX Vite setup page):**

```ts
// source.config.ts — content-source definition (collections)
// Package: fumadocs-mdx (config subpath). Pair with Fumadocs v16 (fumadocs-core / fumadocs-ui v16).
import { defineDocs } from 'fumadocs-mdx/config';

export const docs = defineDocs({
  dir: 'content/docs',
  // `docs` + `meta` collection options are optional; defineDocs is the
  // "combination of meta and doc collections, which is needed for Fumadocs."
  // docs: { /* schema / frontmatter overrides */ },
  // meta: { /* meta.json options */ },
});
```

```ts
// vite.config.ts — the Fumadocs MDX Vite plugin (NON-Next.js path)
import { defineConfig } from 'vite';
import mdx from 'fumadocs-mdx/vite';

export default defineConfig({
  plugins: [
    mdx(),       // compiles content/docs MDX at build time -> a virtual `collections/server` module
    // ... tanstackStart(), react(), cloudflare() per Finding 3.3.2
  ],
});
```

```ts
// lib/source.ts — the runtime Loader (framework-agnostic core)
import { docs } from 'collections/server';     // virtual module emitted by the mdx() Vite plugin
import { loader } from 'fumadocs-core/source';

export const source = loader({
  baseUrl: '/docs',
  source: docs.toFumadocsSource(),
});
```

`defineDocs({ dir })` declares the collection; `mdx()` (from `fumadocs-mdx/vite`) compiles that collection at build time and exposes it as the virtual `collections/server` module; `lib/source.ts` wraps it with `loader()` to produce the framework-agnostic `source` object that `/api/search`, `llms-full.txt`, and the MCP server all consume.
**Source**: [Fumadocs — MDX Vite Setup](https://www.fumadocs.dev/docs/mdx/vite) — Accessed 2026-05-30
**Verification**: The general MDX page corroborates `defineDocs({ dir: 'content/docs' })` from `fumadocs-mdx/config` and the meta+doc collection requirement. [Fumadocs — MDX](https://www.fumadocs.dev/docs/mdx) — Accessed 2026-05-30. The TanStack Start install page independently confirms `lib/source.ts` is the "essential file" the docs route imports via `import { source } from '@/lib/source'`. [Fumadocs — TanStack Start install](https://www.fumadocs.dev/docs/manual-installation/tanstack-start)
**Confidence**: Medium-High (the three files are documented verbatim on the dedicated Vite page; the `frontmatter schema` portion is documented as *optional* collection options on `defineDocs` rather than a separate `defineConfig`/`frontmatterSchema` export — see note below).
**Note on `defineConfig` / frontmatter schema (inference vs documented)**: The prompt anticipated a separate `defineConfig` call and an explicit frontmatter schema. As documented for the Vite path, `source.config.ts` exports only `defineDocs(...)`; a global `defineConfig({ mdxOptions, ... })` export from `fumadocs-mdx/config` exists for MDX-pipeline options (remark/rehype plugins) but is NOT required for the minimal docs setup, and per-collection frontmatter validation is configured through the optional `docs`/`meta` keys on `defineDocs` (Zod schema overrides), not a top-level schema. **Inference**: a `defineConfig`-based `source.config.ts` is the shape used when you need custom MDX options; the minimal Vite path shown above omits it. The exact `defineConfig` signature for v16 was not fetched on the dedicated page in this pass — flagged as residual (see Knowledge Gaps Gap 1 note).
**Framework-path note**: This page is documented as the *Vite* path (vanilla Vite / Waku referenced); the TanStack Start install page explicitly delegates MDX setup to it ("Fumadocs MDX: follow the Vite setup guide"). So the TanStack Start MDX wiring **IS** this Vite triad — `mdx()` co-resident in the same `plugins` array as `tanstackStart()` (Finding 3.3.2). This co-residence (`mdx()` + `tanstackStart()` in one `vite.config.ts`) is **inferred** from the two pages each documenting one half; no single page shows both plugins in one array.

### 3.3 Cloudflare Workers deployment specifics (Wrangler, nodejs_compat)

**Finding 3.3.1 — TanStack Start → Cloudflare Workers is officially supported via the Cloudflare Vite plugin (since 2025-10-24).**
**Evidence**: Cloudflare changelog 2025-10-24: "Cloudflare Vite plugin now supports TanStack Start apps." Scaffold: `npm create cloudflare@latest -- <name> --framework=tanstack-start`.
**Source**: [Cloudflare Changelog — TanStack Start + Vite plugin (2025-10-24)](https://developers.cloudflare.com/changelog/post/2025-10-24-tanstack-start/) — Accessed 2026-05-30
**Confidence**: High (official Cloudflare changelog)

**Finding 3.3.2 — Canonical `wrangler.jsonc` and `vite.config.ts`.**
**Evidence**:
```jsonc
// wrangler.jsonc
{
  "$schema": "node_modules/wrangler/config-schema.json",
  "name": "<YOUR_PROJECT_NAME>",
  "compatibility_date": "2026-05-29",
  "compatibility_flags": ["nodejs_compat"],
  "main": "@tanstack/react-start/server-entry",
  "observability": { "enabled": true }
}
```
```ts
// vite.config.ts
import { defineConfig } from "vite";
import { tanstackStart } from "@tanstack/react-start/plugin/vite";
import { cloudflare } from "@cloudflare/vite-plugin";
import react from "@vitejs/plugin-react";
export default defineConfig({
  plugins: [
    cloudflare({ viteEnvironment: { name: "ssr" } }),
    tanstackStart(),
    react(),
  ],
});
```
Build `npm run build`; deploy `wrangler deploy`; local preview `npm run preview`. **`nodejs_compat` is required.** Static prerender requires TanStack Start **v1.138.0+** via `tanstackStart({ prerender: { enabled: true } })`; prerendered assets serve as static files.
**Source**: [Cloudflare Workers — TanStack Start framework guide](https://developers.cloudflare.com/workers/framework-guides/web-apps/tanstack-start/) — Accessed 2026-05-30
**Confidence**: High (official Cloudflare framework guide, exact config)
**Caveat**: "Prerendering uses local environment variables and bindings; for production data, use remote bindings or set `CLOUDFLARE_INCLUDE_PROCESS_ENV=true` in CI."

### 3.4 Search API + MCP in the Workers runtime (no fs at runtime)

**Finding 3.4.1 — Workers `node:fs` exists with `nodejs_compat` (compat date ≥ 2025-09-01) but is an EPHEMERAL virtual FS — it cannot read your repo's `content/docs/` at runtime.**
**Evidence**: "`node:fs` module is available... with the `nodejs_compat` compatibility flag... when the compatibility date is set to 2025-09-01 or later." Limitations: `fs.watch`/`watchFile` unsupported, `fs.globSync` not implemented, timestamps fixed to Unix epoch, no permissions/ownership. "The file system is ephemeral... files are not persisted across Worker restarts or deployments."
**Source**: [Cloudflare Workers — node:fs](https://developers.cloudflare.com/workers/runtime-apis/nodejs/fs/); [Cloudflare Changelog — Node/Web FS in Workers (2025-08-15)](https://developers.cloudflare.com/changelog/2025-08-15-nodejs-fs/) — Accessed 2026-05-30
**Confidence**: High (official docs + changelog)
**Analysis**: This is the load-bearing runtime constraint. You CANNOT have the Worker read MDX files off disk at request time. The mitigation is exactly the Fumadocs Vite/MDX model: MDX is compiled **at build time** by the Vite plugin and bundled into the JS the Worker serves (§3.2). The `source` Loader the Worker uses at runtime is a bundled in-memory object, not a filesystem walk. Confirm the precise bundling boundary against the Fumadocs Vite/MDX page (Gap), but the architecture is sound: build-time index, runtime in-memory query.

**Finding 3.4.2 — Three viable search architectures on Workers, in order of edge-friendliness.**
**Evidence/Analysis (synthesis)**:
1. **Static export** (`staticGET` + `type:'static'`) — index pre-rendered to JSON at build, served as a static asset, Orama index built **in the browser**. Zero Worker search compute. Best for small/medium corpora. Docs warn against it for large sites (client download cost). [Orama Search]
2. **In-Worker Orama** (`createFromSource` `GET` at `src/routes/api/search.ts`) — index built in-memory inside the Worker isolate on cold start from the bundled `source`. Works under `nodejs_compat`; cost is per-isolate memory + cold-start index build. Viable for a focused corpus; watch isolate memory limits as the corpus grows.
3. **External (Algolia / Orama Cloud)** — build-time `sync()` pushes records to the external index; client queries the service directly. Zero Worker search compute, zero client index download. Best for a large/growing corpus. [Algolia Search]
**Sources**: [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama); [Fumadocs — Algolia Search](https://www.fumadocs.dev/docs/headless/search/algolia) — Accessed 2026-05-30
**Confidence**: Medium-High (the per-architecture mechanics are official; the Worker-isolate-memory caveat for option 2 is analysis, not a cited Fumadocs/Cloudflare statement — flagged in Gaps)

**Finding 3.4.3 — MCP server fits the Workers runtime cleanly via `createMcpHandler()` sharing the build-time index; tool-call logging via Workers Analytics Engine / D1.**
**Evidence/Analysis (synthesis of 1.3, 1.4.2, 3.4.1)**: The MCP server is stateless and reuses the same bundled `source` index — no fs, no DO required. The `/mcp` route lives in the same Worker fetch handler as the docs site and `/api/search`. For the analytics loop, wrap each tool handler to write `{ tool, query, ts, result_count }` to a binding before returning. Workers Analytics Engine and D1 are the edge-native sinks (binding declared in `wrangler.jsonc`).
**Sources**: [Cloudflare Agents — Remote MCP server](https://developers.cloudflare.com/agents/guides/remote-mcp-server/); [Cloudflare Workers — node:fs](https://developers.cloudflare.com/workers/runtime-apis/nodejs/fs/) — Accessed 2026-05-30
**Confidence**: Medium-High (composition; the binding-for-logging is standard Workers practice, not a Fumadocs-specific cite)

**Finding 3.4.4 — Quantified in-Worker Orama ceiling: the Workers isolate limit is a hard 128 MB; Orama's in-memory index measures ≈5 MB per 10k docs (≈0.5 KB/doc), so in-Worker Orama stays viable into the low-hundreds-of-thousands of docs but is bounded by the 128 MB isolate before the millions (GAP 4 — CLOSED).**
**Evidence (the Cloudflare hard limit):** "Memory per isolate | 128 MB." Cloudflare elaborates: "Each isolate can consume up to 128 MB of memory, including the JavaScript heap and WebAssembly allocations. This limit is per-isolate, not per-invocation… When an isolate exceeds 128 MB, the runtime allows in-flight requests to complete, then creates a new isolate for subsequent traffic." The figure is identical on Free and Paid plans. (Separately: compressed Worker bundle limit is 3 MB Free / 10 MB Paid; uncompressed 64 MB — distinct from the runtime 128 MB and a *second* ceiling on how much bundled content/index can ship.)
**Source (limit)**: [Cloudflare Workers — Limits](https://developers.cloudflare.com/workers/platform/limits/) — Accessed 2026-05-30
**Evidence (Orama in-memory index size):** Orama is an entirely in-memory engine. A measured datapoint from the Orama repo: a **100,000-record** index consumed **≈502 MB** without the `internalId` feature and **≈547 MB** with it (post-search, on truly-random data) — i.e. roughly **5 MB per 10,000 records (~0.5 KB per record)** for that synthetic corpus. Maintainer/community discussion confirms "Orama stores everything in memory… at larger scales, when indexing millions of records, users find themselves managing GBs of RAM."
**Source (index size)**: [oramasearch/orama — Issue #573 "Reduce memory usage or build index size?"](https://github.com/oramasearch/orama/issues/573) — Accessed 2026-05-30
**Verification**: A second community thread independently characterises Orama as fully in-memory with GB-scale RAM at millions of records. [oramasearch/orama — Discussion #388](https://github.com/orgs/oramasearch/discussions/388) — Accessed 2026-05-30
**Confidence**: Medium-High for the bounding reasoning; **the Cloudflare 128 MB figure is High (authoritative official limit)**. The per-doc Orama figure is **NOT** an authoritative "max docs in a Worker isolate" number — it is a single synthetic-corpus measurement from a 2023 GitHub issue (predates current Orama optimisations) and a docs corpus's real per-doc footprint differs from random data. **No authoritative Cloudflare-or-Orama-published "max docs for in-Worker Orama" number exists** — this is stated explicitly per the prompt.
**Analysis — the bounding math (clearly labelled inference)**: Treat 128 MB as a hard wall the isolate's *entire* footprint must fit under — the JS heap, the WASM/runtime overhead, the TanStack Start app, the bundled `source`, AND the Orama index all share it. Reserving (generously) ~40-60 MB for the runtime + app + bundled content leaves on the order of ~70-90 MB for the index. At the issue-#573 figure of ≈0.5 KB/doc, that is roughly **140k-180k synthetic docs**; with the safety margin and the fact that *real* prose docs index larger than random tokens, plan the practical in-Worker ceiling at the **low tens of thousands of pages**, not the issue's headline 100k. Concretely:
- **Focused docs corpus (hundreds to a few thousand pages)** — in-Worker Orama (`createFromSource`, §3.4 option 2) is comfortably viable; the index is single-digit MB.
- **Growing corpus (tens of thousands of pages)** — approaching the prudent isolate budget; cold-start index-build time also grows. Migrate to **static export** (browser builds the index — moves the cost off the isolate) or **external Orama Cloud / Algolia** (Findings 2.3.1, 2.3.2 — zero isolate index, build-time `sync()`).
- **Large corpus (>~100k pages / millions of records)** — in-Worker Orama is **not viable** (the index alone approaches/exceeds 128 MB, and the GB-scale warning applies); external search is mandatory.
The 128 MB figure is the *load-bearing hard limit*; the doc-count thresholds are inference from one synthetic measurement and MUST be benchmarked against the actual overdrive.sh corpus before committing to in-Worker search.

---

## 4. Blog support

### 4.1 A blog is a documented pattern, not a turnkey layout

**Finding 4.1 — Fumadocs has NO `<BlogLayout>` / blog page-tree equivalent to the docs experience; a blog reuses the same `fumadocs-mdx` content engine but you bring your own UI (list page + post page).**
**Evidence**: The official "Making a Blog with Fumadocs" guide builds a blog from a `fumadocs-mdx` collection plus hand-written pages — a list page and an individual post page — with no `<BlogLayout>` component shown. (WebFetch summary of the guide: "You **bring your own UI**. There is no turnkey `<BlogLayout>`. You must hand-roll: a list page … individual post pages.") This contrasts directly with the docs experience, which ships `<DocsLayout>` + the docs page tree consumed via the `routes/docs/$.tsx` catch-all (Finding 3.1.1).
**Source**: [Fumadocs — Making a Blog](https://www.fumadocs.dev/blog/make-a-blog) — Accessed 2026-05-30
**Confidence**: Medium (single official guide; corroborated by the absence of any blog-layout export across the UI Search / TanStack Start install pages already surveyed, which enumerate `DocsLayout` / `RootProvider` but no blog component)
**Analysis — the key expectation-setting fact**: For docs, Fumadocs gives you both the content engine AND the UI (`<DocsLayout>`, the page tree, the catch-all route). For a blog, Fumadocs gives you only the content engine — the UI is yours. The budget implication is the same "own the runtime" tradeoff this document applies to the MCP server and search: you write the blog's list/post components, frontmatter schema, and any RSS/OG-image machinery yourself.

### 4.2 Content wiring — the reusable, framework-agnostic part

**Finding 4.2 — The blog is a separate `defineCollections({ type: 'doc', dir: 'content/blog' })` collection (NOT `defineDocs`), fed through `loader()` over `toFumadocsSource(...)`; posts are flat, so no catch-all route is needed.**
**Evidence (collection definition, from the blog guide):**
```ts
// source.config.ts — a SEPARATE collection for the blog (plain `doc`, not defineDocs)
import { defineCollections } from 'fumadocs-mdx/config';
import { z } from 'zod';

export const blogPosts = defineCollections({
  type: 'doc',
  dir: 'content/blog',
  schema: pageSchema.extend({   // extend the built-in page frontmatter
    author: z.string(),
    date: z.string().date().or(z.date()),
  }),
});
```
```ts
// lib/source.ts — wrap the blog collection in the framework-agnostic Loader
import { loader } from 'fumadocs-core/source';
import { toFumadocsSource } from 'fumadocs-mdx/runtime/server';

export const blog = loader({
  baseUrl: '/blog',
  source: toFumadocsSource(blogPosts, []),
});
// access posts via blog.getPages() and blog.getPage([slug])
```
The guide states **"blog posts won't have nested slugs like `/slug1/slug2`"** — i.e. blog posts are flat, so a blog needs no catch-all route, in contrast with the docs `routes/docs/$.tsx` catch-all (Finding 3.1.1).
**Source**: [Fumadocs — Making a Blog](https://www.fumadocs.dev/blog/make-a-blog) — Accessed 2026-05-30
**Verification**: The Collections page confirms the `defineCollections` vs `defineDocs` distinction: `defineCollections({ type: 'doc' | 'meta', dir })` defines a single collection, while **`defineDocs` is the convenience wrapper that combines `doc` + `meta`** ("needed for Fumadocs" docs); the same page shows a blog example using `type: 'doc'` with a custom Zod schema. [Fumadocs — Collections](https://www.fumadocs.dev/docs/mdx/collections) — Accessed 2026-05-30. The framework-agnostic `loader()`-over-collection shape mirrors the docs `lib/source.ts` triad already documented in Finding 3.2.2.
**Confidence**: Medium-High (collection shape + flat-slug fact are documented verbatim in the guide; the `defineCollections`-vs-`defineDocs` distinction is independently confirmed on the Collections page)
**Analysis**: This is the reusable, framework-agnostic half. A blog is just a *second* `loader()`-wrapped collection alongside the docs `source` — `defineDocs` (docs) vs a plain `defineCollections({ type: 'doc' })` (blog, optionally with a `meta` collection if you want ordering JSON). Pin **Fumadocs v16** / the matching `fumadocs-mdx` v16-line per the Version Currency section — the guide itself carries no version numbers, so the version pin is inherited from the rest of this document, not from the blog guide. Re-verify the `toFumadocsSource` import path (`fumadocs-mdx/runtime/server`) against the installed v16 `fumadocs-mdx`; the docs path in Finding 3.2.2 used the `collections/server` virtual module under the Vite plugin, so the exact runtime-import surface may differ between the Vite and standard paths.

### 4.3 The UI side is hand-rolled

**Finding 4.3 — You implement the post-list page, the post page (MDX renderer + TOC), the frontmatter schema, and anything like RSS / OG images yourself; there is no `<BlogLayout>` to absorb this.**
**Evidence**: The guide's post page renders MDX and the TOC with the same primitives the docs use — `<Mdx components={defaultMdxComponents} />` (from `fumadocs-ui/mdx`) and `<InlineTOC items={page.data.toc} />` — styled with Tailwind + Fumadocs design tokens (`text-fd-secondary`, `text-fd-muted-foreground`). The frontmatter schema (`title` / `description` / `author` / `date`) is the `pageSchema.extend({...})` of Finding 4.2. The WebFetch summary explicitly flags **RSS, OG images, and tags as "Not shown"** in the guide — they are left to the implementer.
**Source**: [Fumadocs — Making a Blog](https://www.fumadocs.dev/blog/make-a-blog) — Accessed 2026-05-30
**Confidence**: Medium (single official guide; the list of hand-rolled surfaces is enumerated directly in the guide)
**Analysis**: The blog reuses Fumadocs UI *primitives* (`Mdx`, `InlineTOC`, the `fd-*` design tokens) but not a *layout*. Budget the blog as a from-scratch list page + post page on top of the shared content engine — the same "own the UI/runtime" cost this document applies to search (§2.4) and the MCP server (§1.3). RSS feeds, OG image generation, and tag indexes are net-new work with no Fumadocs scaffold.

### 4.4 Composition with the rest of this stack (load-bearing synthesis for overdrive.sh)

**Finding 4.4 — A blog is a SECOND collection feeding the same build-time index, so it composes with the MCP server, Orama search, and the Workers runtime on identical terms — with the TanStack Start porting caveat carried over.**
**Evidence/Analysis (synthesis of Findings 1.3.1, 2.1.1, 3.1.2, 3.2.2, 3.4.1, 4.2)**:
- **(a) MCP + search index the blog for free.** The blog's `loader()`-wrapped `source` exposes the same `source.getPages()` / `page.data` / `getLLMText(page)` surface (Findings 1.2.1, 1.3.1) that `/api/search` (Orama, Finding 2.1.1) and the custom MCP server (Finding 1.3.1) already consume. Adding blog posts to either index is "point the same indexing pass at a second `source`," not new machinery. To *exclude* the blog (e.g. keep it out of agent answers), scope it out at the `getLLMText`/source-filter boundary — index only the docs `source`, omit `blog`.
- **(b) Workers-compatible on the same terms.** The blog's MDX is compiled into the bundle at build time by the same `fumadocs-mdx` engine (Finding 3.2.2), so the blog needs **no runtime `fs`** either — the load-bearing Workers constraint (Finding 3.4.1) is satisfied identically. The blog's flat posts add bundle weight under the same 3 MB/10 MB compressed-bundle ceiling noted in Finding 3.4.4.
- **(c) TanStack Start caveat carries over (inference).** The collection definition (`source.config.ts` + `lib/source.ts`) is framework-agnostic (Finding 4.2), but the blog guide's list/post **route components are written for the Next.js / standard path** (`app/(home)/blog/page.tsx`, `app/(home)/blog/[slug]/page.tsx`, `fumadocs-mdx/runtime/server`). Porting them to TanStack Start server routes (`createFileRoute(...)` + `createServerFn`, the same shape the docs route uses per Finding 3.1.1) is **inference, not copy-paste** — exactly the documented seam already flagged for the search route in Finding 2.1.2. The maturity caveat (Finding 3.1.2) applies unchanged.
**Sources**: [Fumadocs — Making a Blog](https://www.fumadocs.dev/blog/make-a-blog); [Fumadocs — AI & LLMs](https://www.fumadocs.dev/docs/integrations/llms); [Fumadocs — Orama Search](https://www.fumadocs.dev/docs/headless/search/orama); cross-referenced with Findings 1.3.1, 2.1.1, 3.1.2, 3.2.2, 3.4.1 of this document — Accessed 2026-05-30
**Confidence**: Medium-High (each building block is individually cited above; the composition with the blog as a second collection is analysis, and the TanStack Start port is explicitly labelled inference)

---

## Version Currency (read before copying any code)

- **Fumadocs**: current major is **v16** (blog "Fumadocs v16"), which "refined the API surface, improved compatibility with Vite frameworks, and boosted performance" — i.e. v16 is the version that matters for the **TanStack Start / Vite** path. The framework-agnostic Content→Core→UI split progressed across **v14 → v15 → v16** (`fumadocs-ui` was at 15.0.14 on npm during the v15 line). v16 also relocated server exports (`fumadocs-core/content/github`, `fumadocs-core/page-tree`, `fumadocs-core/toc`). **Pin v16+** and verify the exact `source.config.ts` / Vite-plugin API against the v16 docs, since v16 carried breaking changes. Sources: [Fumadocs v16 blog](https://www.fumadocs.dev/blog/v16); [Fumadocs releases](https://github.com/fuma-nama/fumadocs/releases); [fumadocs-ui npm](https://www.npmjs.com/package/fumadocs-ui).
- **MCP TypeScript SDK**: the published package is **`@modelcontextprotocol/sdk`** (subpath imports for `McpServer`, `StreamableHTTPServerTransport`, `StdioServerTransport`). `server.registerTool(name, { description, inputSchema: zod }, handler)` is current. (An earlier fetch paraphrased the package as `@modelcontextprotocol/server`/`/node` — that is NOT the published name; use `@modelcontextprotocol/sdk`.) Sources: [@modelcontextprotocol/sdk npm](https://www.npmjs.com/package/@modelcontextprotocol/sdk); [typescript-sdk repo](https://github.com/modelcontextprotocol/typescript-sdk).
- **TanStack Start on Cloudflare**: officially supported via `@cloudflare/vite-plugin` since **2025-10-24**; static prerender needs TanStack Start **v1.138.0+**. Both move fast — re-check the framework guide before pinning.

---

## Recommended Setup for overdrive.sh

> **Updated 2026-05-30 to the resolved § Decision (Next.js + OpenNext).** The prior TanStack Start
> recommendation is preserved directly below this block as the rejected-alternative record.

A concrete, edge-first stack on the **chosen** path (Next.js App Router + RSC + OpenNext on Cloudflare
Workers):

1. **Framework**: Fumadocs **v16** (`fumadocs-core` + `fumadocs-ui`) on **Next.js (App Router, RSC)**,
   MDX via **`createMDX()` from `fumadocs-mdx/next`** wrapping `next.config`. Docs catch-all at
   `app/docs/[[...slug]]/page.tsx`, `lib/source.ts` over the Next-emitted source. Scaffold via
   `npm create cloudflare@latest -- <name> --framework=next --platform=workers`. Use **Next 15 or 16**.
2. **Deploy**: Cloudflare Workers via **`@opennextjs/cloudflare`** + Wrangler. `compatibility_flags:
   ["nodejs_compat"]`. **Never `export const runtime = 'edge'`** — OpenNext runs the Workers Node
   runtime. Wire an **R2 incremental cache binding only if you use `revalidate`** (a build-time/SSG
   docs site needs none). `next/image` → `unoptimized` or a custom loader.
3. **Search**: start with **in-Worker Orama** (`export const { GET } = createFromSource(source)` at
   `app/api/search/route.ts`, Node runtime) for a focused corpus; plan a migration to **Orama Cloud or
   Algolia** (build-time `sync()`) if the corpus + isolate memory grow (Finding 3.4.2, 128 MB ceiling
   Finding 3.4.4). Static-export mode is the fallback if you want zero runtime search compute.
4. **MCP server**: a **stateless** server reusing the same build-time index, exposed as a **Next route
   handler** (`app/mcp/route.ts`, Node runtime) over Streamable HTTP — or a separate dedicated Worker.
   Tools: `search_docs(query)` (reuse the Orama index) and `get_doc(url)` (return `getLLMText(page)`).
   One build-time index feeds `/api/search`, `llms-full.txt`, AND the MCP server (Finding 1.3.1).
   (Cloudflare Agents `createMcpHandler()` from Finding 1.4.2 is still usable, wired inside the route
   handler.)
5. **Analytics loop**: wrap each MCP tool handler to log `{ tool, query, ts, result_count }` to a
   **Workers Analytics Engine** (or D1) binding before returning (Finding 3.4.3).
6. **Free fallback / interop**: also publish `llms.txt` + `llms-full.txt` + per-page `.md` (Findings
   1.1.1, 1.2.1) so non-MCP agents and generic tools (LangChain `mcpdoc`) work without your custom
   server.
7. **Optional, now unlocked**: `fumadocs-openapi` interactive API playground — works on this RSC-native
   path (it does **not** work on the rejected TanStack/React Router paths; Discussion #3153).

---

### (Rejected alternative, retained) Original TanStack Start setup

A concrete, edge-first stack consistent with "own the runtime + ship an MCP server + log tool calls + Cloudflare Workers + TanStack Start":

1. **Framework**: Fumadocs **v16** (`fumadocs-core` + `fumadocs-ui`) on **TanStack Start**, MDX via the **Fumadocs MDX Vite plugin** (`source.config.ts` + `lib/source.ts`). Accept the maintainer's "early" caveat (Finding 3.1.2) — prefer the **TanStack Start SPA template** as the baseline, which is both maintainer-recommended and the most Workers-friendly shape.
2. **Deploy**: Cloudflare Workers via `@cloudflare/vite-plugin` + `wrangler` (Finding 3.3.2). `compatibility_flags: ["nodejs_compat"]`, `main: "@tanstack/react-start/server-entry"`, enable `tanstackStart({ prerender: { enabled: true } })` for the static-leaning docs surface.
3. **Search**: start with **in-Worker Orama** (`createFromSource` at `src/routes/api/search.ts`) for a focused corpus; plan a migration to **Orama Cloud or Algolia** (build-time `sync()`) if the corpus + isolate memory grow (Finding 3.4.2). Static-export mode is the fallback if you want zero runtime search compute and accept the client index download.
4. **MCP server**: a **stateless** server via Cloudflare Agents **`createMcpHandler()`** exposing `/mcp` (Streamable HTTP) in the same Worker (Findings 1.4.2, 3.4.3). Tools: `search_docs(query)` (reuse the Orama index) and `get_doc(url)` (return `getLLMText(page)`). One build-time index feeds `/api/search`, `llms-full.txt`, AND the MCP server (Finding 1.3.1).
5. **Analytics loop**: wrap each MCP tool handler to log `{ tool, query, ts, result_count }` to a **Workers Analytics Engine** (or D1) binding before returning (Finding 3.4.3).
6. **Free fallback / interop**: also publish `llms.txt` + `llms-full.txt` + per-page `.md` (Findings 1.1.1, 1.2.1) so non-MCP agents and generic tools (LangChain `mcpdoc`) work without your custom server.

**The one blocker to weigh**: TanStack Start is maintainer-flagged as immature (Finding 3.1.2). It is not unsupported — both Fumadocs (TanStack template) and Cloudflare (official Vite-plugin guide) document the path — but it is the least battle-tested leg of the stack. If docs-site robustness outweighs the "agent-native framework" preference, the maintainer's own ranking is Next.js > React Router > TanStack Start. React Router is the middle-ground that keeps you off Next.js/Vercel while being more mature than TanStack Start.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Fumadocs — TanStack Start install | fumadocs.dev | High (0.9, official_publication) | official | 2026-05-30 | Y |
| Fumadocs — AI & LLMs | fumadocs.dev | High (0.9) | official | 2026-05-30 | Y |
| Fumadocs — Orama Search | fumadocs.dev | High (0.9) | official | 2026-05-30 | Y |
| Fumadocs — Algolia Search | fumadocs.dev | High (0.9) | official | 2026-05-30 | N (single) |
| Fumadocs v16 blog | fumadocs.dev | High (0.9) | official | 2026-05-30 | Y |
| Fumadocs GitHub Discussion #2442 | github.com | Medium-High (0.8) | industry/maintainer | 2026-05-30 | N (single thread) |
| Fumadocs releases / npm | github.com / npmjs.com | Medium-High (0.8) | official artifact | 2026-05-30 | Y |
| Cloudflare — TanStack Start guide | developers.cloudflare.com | High (0.9) | official | 2026-05-30 | Y |
| Cloudflare — TanStack changelog | developers.cloudflare.com | High (0.9) | official | 2026-05-30 | Y |
| Cloudflare — node:fs + changelog | developers.cloudflare.com | High (0.9) | official | 2026-05-30 | Y |
| Cloudflare Agents — Remote MCP server | developers.cloudflare.com | High (0.9) | official | 2026-05-30 | Y |
| Cloudflare Blog — Streamable HTTP MCP | blog.cloudflare.com | High (0.9) | official | 2026-05-30 | Y |
| MCP — Transports spec | modelcontextprotocol.io | High (0.9, official_publication) | official | 2026-05-30 | Y |
| MCP TypeScript SDK — server.md / npm | github.com / npmjs.com | High (0.9) | official | 2026-05-30 | Y |
| LangChain mcpdoc | github.com | Medium-High (0.8) | industry | 2026-05-30 | N (single) |
| Fumadocs — MDX Vite Setup | fumadocs.dev | High (0.9) | official | 2026-05-30 | Y |
| Fumadocs — MDX (general) | fumadocs.dev | High (0.9) | official | 2026-05-30 | Y |
| Fumadocs — UI Search | fumadocs.dev | High (0.9) | official | 2026-05-30 | Y |
| Fumadocs — Orama Cloud Search | fumadocs.dev | High (0.9) | official | 2026-05-30 | Y |
| TanStack Start — Server Routes | tanstack.com | High (0.9, official_publication) | official | 2026-05-30 | Y |
| Cloudflare — Workers Limits | developers.cloudflare.com | High (0.9) | official | 2026-05-30 | Y |
| oramasearch/orama — Issue #573 / Discussion #388 | github.com | Medium (0.6) | community/issue | 2026-05-30 | Y (two threads) |
| Orama Docs — Orama Cloud | docs.orama.com | High (0.9, official_publication) | official | 2026-05-30 | N (corroborating) |
| Fumadocs — Making a Blog | fumadocs.dev | High (0.9, official_publication) | official | 2026-05-30 | Y (§4) |
| Fumadocs — Collections | fumadocs.dev | High (0.9, official_publication) | official | 2026-05-30 | Y (§4.2) |
| Fumadocs Discussion #2442 — TanStack fully static | github.com | Medium-High (0.8) | maintainer thread | 2026-05-30 | Y (Decision) |
| Fumadocs Discussion #2880 — upgrade 16.0→16.4 on TanStack | github.com | Medium-High (0.8) | maintainer thread | 2026-05-30 | Y (Decision) |
| Fumadocs Discussion #2969 — TanStack base path | github.com | Medium-High (0.8) | maintainer thread | 2026-05-30 | Y (Decision) |
| Fumadocs Discussion #3153 — OpenAPI RSC-only | github.com | Medium-High (0.8) | maintainer thread | 2026-05-30 | Y (Decision) |
| OpenNext — Cloudflare adapter | opennext.js.org | High (0.9, official_publication) | official | 2026-05-30 | Y (Decision) |
| Cloudflare — Next.js framework guide | developers.cloudflare.com | High (0.9) | official | 2026-05-30 | Y (Decision) |
| workers-sdk #11825 — workerd SSR duplicate React | github.com | Medium-High (0.8) | official issue | 2026-05-30 | N (single, React Router note) |

Reputation (updated 2026-05-30, +§4 blog): High: ~21 (~84%) | Medium-High: 3 | Medium: 1 (orama issue threads — flagged as a single synthetic measurement, not authoritative) | Excluded: 0 | **Avg ≈ 0.86** (meets ≥0.80 target).

## Knowledge Gaps

### Gap 1: Exact `source.config.ts` + Fumadocs MDX **Vite** plugin code — CLOSED (Finding 3.2.2)
**Resolved**: The dedicated Fumadocs MDX Vite setup page provides all three files verbatim: `source.config.ts` (`defineDocs({ dir: 'content/docs' })` from `fumadocs-mdx/config`), `vite.config.ts` (`mdx()` from `fumadocs-mdx/vite`), and `lib/source.ts` (`loader()` over `docs.toFumadocsSource()`). See Finding 3.2.2.
**Residual (minor)**: The optional `defineConfig({ mdxOptions, ... })` export (used only when custom remark/rehype MDX-pipeline options are needed) was not separately fetched in detail — the minimal Vite path does not require it, and per-collection frontmatter validation is via the optional `docs`/`meta` Zod schema keys on `defineDocs`. The exact v16 `defineConfig` signature should be confirmed against the MDX config-reference page if custom MDX options are needed. Additionally, the co-residence of `mdx()` + `tanstackStart()` in one `plugins` array is inferred (each page documents one plugin), not shown together on a single official page.

### Gap 2: Search route (TanStack Start) + in-Worker Orama + Orama Cloud client wiring — CLOSED (Findings 2.1.2, 2.3.2)
**Resolved**: The TanStack Start search route is `const server = createFromSource(source, { language: 'english' })` (Fumadocs install page), adapted to TanStack's `createFileRoute(...).server.handlers.GET` (TanStack server-routes guide) — the one genuine divergence from the Next.js `export const { GET }` default. The handler binding is inferred (both halves official; their composition is not on one page). Orama Cloud wiring: build-time `sync()` from `fumadocs-core/search/orama-cloud` + client `useDocsSearch({ type: 'orama-cloud', client })` with the `@orama/core` `OramaCloud` client (Finding 2.3.2).
**Residual (minor)**: The exact `new OramaCloud({...})` constructor options were paraphrased, not fully quoted; the `@orama/core` vs `@oramacloud/client` package-name ambiguity should be pinned against the installed SDK version.

### Gap 3: `<RootProvider>` search-config override + custom `SearchDialog` — CLOSED (Finding 2.4.2)
**Resolved**: The Fumadocs UI Search page gives the full `search={{ enabled, preload, hotKey, links, options, SearchDialog }}` prop shape, the default fetch client (`useDocsSearch({ type: 'fetch' })`), and the custom-dialog swap via `search.SearchDialog` composing primitives from `fumadocs-ui/components/dialog/search`. See Finding 2.4.2.

### Gap 4: In-Worker Orama isolate-memory ceiling (quantified) — CLOSED (Finding 3.4.4)
**Resolved**: Cloudflare's authoritative per-isolate limit is **128 MB** (Workers Limits page). Orama is fully in-memory; a synthetic 100k-record index measured ≈502-547 MB (≈0.5 KB/record) per oramasearch/orama issue #573. **No authoritative "max docs for in-Worker Orama" number exists** — Finding 3.4.4 derives the practical ceiling (low tens of thousands of real pages for the index, after reserving ~40-60 MB for runtime+app+bundle within the 128 MB wall) as clearly-labelled inference, and recommends benchmarking the actual corpus before committing to in-Worker search.
**Residual**: The per-doc Orama figure is one 2023 synthetic measurement; real prose-doc footprint and current Orama optimisations differ. Benchmark required.

## Conflicting Information

### Conflict 1: Package name for the MCP SDK HTTP transport
**Position A**: One WebFetch summary rendered the SDK as `@modelcontextprotocol/server` + `@modelcontextprotocol/node`. — Source: paraphrase of [typescript-sdk server.md](https://github.com/modelcontextprotocol/typescript-sdk/blob/main/docs/server.md).
**Position B**: The published npm package is a single `@modelcontextprotocol/sdk` with subpath imports. — Source: [@modelcontextprotocol/sdk npm](https://www.npmjs.com/package/@modelcontextprotocol/sdk).
**Assessment**: Position B is authoritative (the npm registry is the canonical package-name source). Position A is a summarization artifact. **Use `@modelcontextprotocol/sdk`.** (On Cloudflare Workers, prefer the Agents SDK `createMcpHandler()` over raw SDK transport anyway — Finding 1.4.2.)

## Confidence / Caveats

**Overall confidence: Medium-High.** Each of the three concerns rests on official_publication-tier sources (Fumadocs, Cloudflare, MCP). The two soft spots: (1) the TanStack-Start maturity caveat rests on a single (maintainer) GitHub thread — high-trust author, single source; (2) the *composition* findings (custom MCP server reusing the Fumadocs index; in-Worker Orama memory) are analysis built on individually-cited primitives, not a single end-to-end official tutorial — there is **no official "Fumadocs MCP server on Cloudflare Workers with TanStack Start" walkthrough**; this exact stack is assembled from three official paths that each stop at their own boundary.

## Full Citations
[1] Fumadocs. "TanStack Start (Manual Installation)". fumadocs.dev. https://www.fumadocs.dev/docs/manual-installation/tanstack-start. Accessed 2026-05-30.
[2] Fumadocs. "AI & LLMs". fumadocs.dev. https://www.fumadocs.dev/docs/integrations/llms. Accessed 2026-05-30.
[3] Fumadocs. "Orama Search (Headless)". fumadocs.dev. https://www.fumadocs.dev/docs/headless/search/orama. Accessed 2026-05-30.
[4] Fumadocs. "Algolia Search (Headless)". fumadocs.dev. https://www.fumadocs.dev/docs/headless/search/algolia. Accessed 2026-05-30.
[5] Fumadocs. "Fumadocs v16". fumadocs.dev/blog. https://www.fumadocs.dev/blog/v16. Accessed 2026-05-30.
[6] fuma-nama. "Fumadocs + Tanstack Start fully static — Discussion #2442". GitHub. https://github.com/fuma-nama/fumadocs/discussions/2442. Accessed 2026-05-30.
[7] Fumadocs. "Releases". GitHub. https://github.com/fuma-nama/fumadocs/releases. Accessed 2026-05-30.
[8] Cloudflare. "TanStack Start — Workers Framework Guide". developers.cloudflare.com. https://developers.cloudflare.com/workers/framework-guides/web-apps/tanstack-start/. Accessed 2026-05-30.
[9] Cloudflare. "Build TanStack Start apps with the Cloudflare Vite plugin (Changelog, 2025-10-24)". developers.cloudflare.com. https://developers.cloudflare.com/changelog/post/2025-10-24-tanstack-start/. Accessed 2026-05-30.
[10] Cloudflare. "node:fs — Workers runtime APIs". developers.cloudflare.com. https://developers.cloudflare.com/workers/runtime-apis/nodejs/fs/. Accessed 2026-05-30.
[11] Cloudflare. "The Node.js and Web File System APIs in Workers (Changelog, 2025-08-15)". developers.cloudflare.com. https://developers.cloudflare.com/changelog/2025-08-15-nodejs-fs/. Accessed 2026-05-30.
[12] Cloudflare. "Build a Remote MCP server — Agents docs". developers.cloudflare.com. https://developers.cloudflare.com/agents/guides/remote-mcp-server/. Accessed 2026-05-30.
[13] Cloudflare. "Bringing streamable HTTP transport and Python to MCP servers (Blog)". blog.cloudflare.com. https://blog.cloudflare.com/streamable-http-mcp-servers-python/. Accessed 2026-05-30.
[14] Model Context Protocol. "Transports (Specification 2025-03-26)". modelcontextprotocol.io. https://modelcontextprotocol.io/specification/2025-03-26/basic/transports. Accessed 2026-05-30.
[15] Model Context Protocol. "TypeScript SDK — server.md". GitHub. https://github.com/modelcontextprotocol/typescript-sdk/blob/main/docs/server.md. Accessed 2026-05-30.
[16] Model Context Protocol. "@modelcontextprotocol/sdk". npm. https://www.npmjs.com/package/@modelcontextprotocol/sdk. Accessed 2026-05-30.
[17] LangChain. "mcpdoc — Expose llms-txt to IDEs". GitHub. https://github.com/langchain-ai/mcpdoc. Accessed 2026-05-30.
[18] Fumadocs. "MDX — Vite Setup". fumadocs.dev. https://www.fumadocs.dev/docs/mdx/vite. Accessed 2026-05-30. (Gap 1)
[19] Fumadocs. "MDX". fumadocs.dev. https://www.fumadocs.dev/docs/mdx. Accessed 2026-05-30. (Gap 1, corroborating)
[20] Fumadocs. "UI — Search". fumadocs.dev. https://www.fumadocs.dev/docs/ui/search. Accessed 2026-05-30. (Gap 3)
[21] Fumadocs. "Orama Cloud Search (Headless)". fumadocs.dev. https://www.fumadocs.dev/docs/headless/search/orama-cloud. Accessed 2026-05-30. (Gap 2/3)
[22] TanStack. "Server Routes — TanStack Start (React)". tanstack.com. https://tanstack.com/start/latest/docs/framework/react/guide/server-routes. Accessed 2026-05-30. (Gap 2)
[23] Cloudflare. "Limits — Workers Platform". developers.cloudflare.com. https://developers.cloudflare.com/workers/platform/limits/. Accessed 2026-05-30. (Gap 4)
[24] H4ad / oramasearch. "Reduce memory usage or build index size? — Issue #573". GitHub. https://github.com/oramasearch/orama/issues/573. Accessed 2026-05-30. (Gap 4)
[25] oramasearch. "From Elastic Search to Orama — Discussion #388". GitHub. https://github.com/orgs/oramasearch/discussions/388. Accessed 2026-05-30. (Gap 4, corroborating)
[26] Orama. "Orama Cloud". docs.orama.com. https://docs.orama.com/cloud. Accessed 2026-05-30. (Gap 2/3, corroborating @orama/core client)
[27] Fumadocs. "Making a Blog with Fumadocs". fumadocs.dev/blog. https://www.fumadocs.dev/blog/make-a-blog. Accessed 2026-05-30. (§4)
[28] Fumadocs. "Collections (MDX)". fumadocs.dev. https://www.fumadocs.dev/docs/mdx/collections. Accessed 2026-05-30. (§4.2)
[29] fuma-nama. "Fumadocs + Tanstack Start fully static — Discussion #2442". GitHub. https://github.com/fuma-nama/fumadocs/discussions/2442. Accessed 2026-05-30. (Decision; full thread fetched via GraphQL)
[30] fuma-nama. "Upgrade to latest Fumadocs and TanStack Start — Discussion #2880". GitHub. https://github.com/fuma-nama/fumadocs/discussions/2880. Accessed 2026-05-30. (Decision)
[31] fuma-nama. "Fumadocs + Tanstack Start Base Path — Discussion #2969". GitHub. https://github.com/fuma-nama/fumadocs/discussions/2969. Accessed 2026-05-30. (Decision)
[32] fuma-nama. "fumadocs-openapi support for TanStack Start (non-RSC SSR)? — Discussion #3153". GitHub. https://github.com/fuma-nama/fumadocs/discussions/3153. Accessed 2026-05-30. (Decision; RSC feature gate)
[33] OpenNext. "Cloudflare". opennext.js.org. https://opennext.js.org/cloudflare. Accessed 2026-05-30. (Decision)
[34] Cloudflare. "Next.js — Workers Framework Guide". developers.cloudflare.com. https://developers.cloudflare.com/workers/framework-guides/web-apps/nextjs/. Accessed 2026-05-30. (Decision)
[35] Cloudflare. "@cloudflare/vite-plugin: SSR causes 'Invalid hook call' (module duplication) — workers-sdk Issue #11825". GitHub. https://github.com/cloudflare/workers-sdk/issues/11825. Accessed 2026-05-30. (React Router rejected-alternative note)

## Research Metadata
Duration: ~1.5 sessions (initial + gap-closure pass) + §4 blog-support addendum | Examined: ~26 sources | Cited: 28 | Cross-refs: most claims 2-3 sources | Confidence: High ~70%, Medium ~30%, Low 0% | Output: docs/research/platform/fumadocs-mcp-search-cloudflare-tanstack-research.md
