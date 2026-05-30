# ADR-0055 — docs-platform: MCP server is a same-Worker Next route handler (`app/mcp/route.ts`, Node runtime)

## Status

Accepted. 2026-05-30. Decision-makers: Morgan (proposing); user (GUIDE-mode
decision D-A, pass 1 → locked pass 2). Tags: docs-platform, website,
overdrive.sh, mcp, integration-invariant, application-arch.

**Scope note**: This ADR governs the **overdrive.sh website** (`website/`
subtree, TypeScript/Next.js), which is architecturally **independent** of the
Rust platform. It does not touch any `crates/` decision and is exempt from the
Rust crate-class / dst-lint / nextest / cargo-mutants gates (DISCUSS C-5).
Recorded in the shared ADR sequence with a docs-platform-scoped title so the
brief's ADR index resolves it; see § "On the shared ADR namespace" below.

References the docs-platform DISCUSS output
(`docs/feature/docs-platform/feature-delta.md` US-05, US-06, constraints C-2,
C-4, C-7) and the research record
(`docs/research/platform/fumadocs-mcp-search-cloudflare-tanstack-research.md`
§ Decision, Findings 1.3.1, 1.4.2, 3.4.3).

## Context

US-05 (J-DOCS-002) requires an MCP endpoint at `/mcp` exposing two tools —
`search_docs(query)` and `get_doc(url)` — that a developer's coding agent
configures once and queries to ground its answers in current Overdrive docs.

The strategic invariant of the whole feature (DISCUSS C-4, the journey's
`docs_content_index` shared artifact) is **one build-time index, multiple
consumers**: the browser search dialog (US-03), the MCP `search_docs` (US-05),
and the `llms.txt`/`llms-full.txt` export (US-04) must all read the SAME
`source.getPages()` index and the SAME `getLLMText(page)` primitive. Divergence
between what a human searches and what an agent searches is an integration
failure, not a cosmetic one. The single highest-leverage architectural question
for US-05 is therefore: **where does the MCP handler run relative to that
index?**

The stack is locked (DISCUSS C-1): Next.js (App Router / RSC) on Cloudflare
Workers via `@opennextjs/cloudflare`, Node runtime everywhere (C-2 — never
`export const runtime = 'edge'`). MDX is compiled into the bundle at build time
(C-3 — no runtime `fs`). The research records two viable MCP topologies on this
path (§ Decision → "§1 carries over"; Finding 1.4.2):

- A **Next route handler** in the same Worker (`app/mcp/route.ts`), sharing the
  one in-process `source` index that already feeds `/api/search` and
  `llms-full.txt`.
- A **separate dedicated Worker** for `/mcp`, with its own copy of the index.

The research explicitly flags that with OpenNext, "bolting an arbitrary
non-Next route onto the same Worker is awkward, so the route handler … is
preferred over the §3.4.3 'same-Worker fetch route' framing" (§ Decision).

## Decision

The MCP server is a **same-Worker Next route handler at `website/app/mcp/route.ts`,
on the default Node runtime** (no `runtime = 'edge'`, per C-2). It imports the
ONE build-time `source` index (`lib/source.ts`) and the ONE `getLLMText`
primitive (`lib/get-llm-text.ts`) in-process — the same modules `/api/search`
and the llms exports import.

- `search_docs(query)` delegates to the shared `lib/search.ts` seam's
  `searchIndex(query)` (ADR-0057) — the identical function the `/api/search`
  handler calls. It returns ranked `{ title, url, excerpt }`.
- `get_doc(url)` resolves the URL against `source`, returns `getLLMText(page)`,
  and MUST be **byte-identical** to the per-page `.md` export for the same page
  (US-05 AC; DISCUSS `page_llm_text` shared-artifact contract). A URL that does
  not resolve returns an honest not-found tool result, never fabricated content.
- The transport is Streamable HTTP, **stateless** (no `sessionIdGenerator` /
  no Durable Object — Finding 1.4.2: a docs-query server needs no per-session
  state).

**Implementation latitude (not a topology change)**: the handler MAY use
`createMcpHandler()` from the Cloudflare Agents SDK inside the route body if the
raw MCP TypeScript SDK Streamable-HTTP transport proves fiddly. Either way the
tool bodies call the shared seams; the SDK choice is internal to the handler and
does not affect this ADR's "one index, in-process" decision. Tool input schemas
are declared with `zod`.

The same handler is the host for the best-effort tool-call logging wrapper
(ADR-0056) — logging is layered around the tool bodies in this same file.

## Alternatives considered

### Alternative A — Separate dedicated Worker for `/mcp` (REJECTED — the pass-1 alternative)

A standalone Worker serving only `/mcp`, deployed alongside the docs Worker.

- **Why considered**: clean separation of the agent surface from the
  human-facing site; independent scaling/deploy of the MCP endpoint;
  the research lists it as co-equal viable.
- **Why rejected**: it would hold its **own copy** of the `source` index, built
  from the same content but in a second build/bundle. That is precisely the
  C-4 divergence risk the whole feature is organised against — two indexes that
  are "supposed to be the same" drift the first time one is rebuilt and the
  other is not. It also adds a second deploy unit, a second cold-start surface,
  and a second place the `SITE_ORIGIN` (ADR — D-F) and `getLLMText` primitive
  must be wired identically. For a single-team, single-corpus docs site
  (team ≪ 10; the modular-monolith default applies), a second Worker buys
  independent deployability the feature does not need and pays for it in the
  one quality attribute (no-divergence) that matters most here. The build-time
  one-index enforcement assertion (ADR-0058) cannot span two separate build
  outputs cheaply.

### Alternative B — `mcpdoc` (LangChain) pointed at `/llms.txt` (REJECTED)

Use the off-the-shelf `mcpdoc` MCP server consuming the published `llms.txt`,
zero bespoke server code (research Finding 1.3.2).

- **Why considered**: zero build; "what you'd get for free."
- **Why rejected**: it exposes only a generic `fetch_docs`-over-URLs tool — no
  bespoke `search_docs` ranking and, decisively, **no tool-call analytics loop**
  (US-06 / J-DOCS-003, the strategic differentiator). It also runs outside our
  runtime, so we cannot honor the best-effort logging contract (C-7). It is the
  correct *fallback* for third parties who want to consume our `llms.txt`, not
  the *primary* surface we own.

### Alternative C — Edge-runtime route handler (REJECTED — violates C-2)

- **Why rejected**: OpenNext does not support `export const runtime = 'edge'`;
  the adapter manages the runtime (research § Decision → OpenNext constraints).
  Hard constraint, not a trade-off.

## Consequences

**Positive**

- **Strongest possible no-divergence guarantee for C-4.** The browser search,
  the MCP `search_docs`, and the llms export are literally three importers of
  one in-process module graph. There is no second index to drift.
- One deploy unit, one cold-start surface, one `SITE_ORIGIN` wiring.
- `get_doc` byte-identity with the `.md` export is trivially satisfiable because
  both call the same `getLLMText` — the build-time assertion (ADR-0058) can
  verify it.
- Statelessness sidesteps Durable Object billing/complexity entirely.

**Negative / accepted trade-offs**

- The MCP endpoint shares the docs Worker's 128 MB isolate budget and the
  3 MiB/10 MiB compressed-bundle ceiling (DISCUSS C-8) with the search index and
  the app. This is the same ceiling ADR-0057 governs; for the current corpus it
  is comfortable. If the corpus ever forces external search (ADR-0057's
  migration trigger), the MCP `search_docs` follows the seam to the external
  index automatically — no topology change here.
- No independent scaling of `/mcp` vs the site. Accepted: the site and the
  agent surface scale together at this stage; revisiting is a single-file change
  (move the route into its own Worker) if a future load profile demands it —
  a property, not a promised follow-up.

## On the shared ADR namespace

A dedicated `website/`-scoped ADR namespace (e.g. `website/docs/adr-*`) would
read more cleanly than mixing docs-platform decisions into the Rust platform's
`adr-00NN` sequence. This ADR keeps the shared sequence anyway, because the
brief's `## ADR index` and the downstream DEVOPS/DISTILL waves look there, and a
clearly docs-platform-scoped title disambiguates it from Rust-platform ADRs at a
glance. Recommendation surfaced, not acted on; revisit if the website accrues
enough ADRs to warrant its own index.
