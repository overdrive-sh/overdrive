<!-- markdownlint-disable MD024 -->
# Feature Delta — docs-platform (DISCUSS wave)

**Feature**: `docs-platform` — the overdrive.sh documentation / marketing /
agent-discovery site.
**Wave**: DISCUSS (Luna, nw-product-owner). **JTBD**: ON (Decision 4 = Yes).
**Date**: 2026-05-30.
**Stack (DECIDED, not re-litigated)**: Fumadocs on Next.js (App Router, RSC),
deployed to Cloudflare Workers via `@opennextjs/cloudflare` (OpenNext).

This single narrative is the DISCUSS output of record. It contains: JTBD
analysis, scope assessment + Elephant Carpaccio split, the two-arc journey,
the story map + walking skeleton, the LeanUX user stories with embedded
acceptance criteria, outcome KPIs, DoR validation, and wave decisions.
Machine artifacts: `slices/slice-NN-*.md` (one per slice). SSOT updates:
`docs/product/jobs.yaml` (new J-DOCS-* jobs), `docs/product/journeys/docs-platform.yaml`,
`docs/product/personas/{priya-evaluator,maya-agent-developer}.yaml`.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

Run BEFORE journey-visualization investment, per the early gate.

**Verdict: OVERSIZED → split into 8 thin end-to-end slices. User-confirmed
split (the wizard decision pre-confirmed the slice arc and walking skeleton).**

Oversized signals tripped (≥2 required; 4 tripped):

- **>10 stories likely**: the feature spans landing + docs + blog + search +
  MCP + analytics + deploy — 8 stories.
- **Multiple independent shippable outcomes**: a live URL, a browsable docs
  tree, search, an LLM export, an MCP server, an analytics loop, a blog, and a
  landing page can each ship and deliver value on their own.
- **Cross-context complexity ≥3 technologies**: Next.js/RSC, OpenNext,
  Cloudflare Workers, Orama, MCP, Fumadocs MDX pipeline.
- **Walking skeleton needs an end-to-end deploy** before any feature is real.

### The split — 8 thin slices (each ≤1 day, each end-to-end, each user-visible)

| Slice | Name | Story | Job | Walking skeleton? |
|---|---|---|---|---|
| 01 | Skeleton Deploy | US-01 | J-DOCS-001 | **YES — walking skeleton** |
| 02 | Real Docs Content + Nav | US-02 | J-DOCS-001 | |
| 03 | Search | US-03 | J-DOCS-001 | |
| 04 | LLM Content Export | US-04 | J-DOCS-002 | |
| 05 | MCP Server (search_docs + get_doc) | US-05 | J-DOCS-002 | |
| 06 | MCP Tool-Call Analytics Loop | US-06 | J-DOCS-003 | |
| 07 | Blog | US-07 | J-DOCS-001 | |
| 08 | Landing Page | US-08 | J-DOCS-001 | |

**Walking skeleton = Slice 01.** Thinnest end-to-end slice: minimal
Fumadocs+Next+OpenNext site, ONE docs page, live on Cloudflare Workers. The
dogfood moment is "the page is reachable at a real URL."

**Carpaccio taste tests applied**: every slice (a) ships end-to-end (not a
technical layer), (b) is ≤1 day, (c) has a named learning hypothesis, (d) has
IN/OUT scope, (e) delivers a user-visible behaviour. **No infra-only slices** —
slice 01's value is "a real reachable URL" (a human can open it).

**Estimated total**: ~8 days across 8 independent deliverables. Prefer 8 tiny
shippable slices over 1 big one.

---

## Wave: DISCUSS / [WHY] JTBD Analysis

This feature is a **different KIND** of feature than the platform-internal
jobs in `docs/product/jobs.yaml` (J-PLAT-*, J-OPS-* are about the Rust
platform itself). NONE covered a docs consumer. We **bootstrapped** three new
jobs (now appended to `jobs.yaml` with a changelog entry; existing entries
untouched).

### J-DOCS-001 — Human evaluator/adopter finds an authoritative answer fast

> **When** I'm evaluating or adopting Overdrive and land on overdrive.sh with a
> concrete question (concept, how-to, or API/CLI detail), **I want to** find
> the authoritative answer on the site in seconds — via search or browsing —
> without reading source or asking in Slack, **so I can** decide/proceed on my
> own and so my first impression is "serious, well-documented platform."

- **Functional**: get the answer (concept / how-to / API).
- **Emotional**: feel confident the project is serious; not be stranded mid-adoption.
- **Social**: tell her team "I evaluated it, here's what it does" with evidence.
- **Four forces** — Push: tired of reverse-engineering new infra from source;
  asking in Slack is slow. Pull: a dense, searchable docs site that answers in
  seconds. Anxiety: "the search will return marketing fluff / the docs will
  describe features that don't exist." Habit: defaults to cloning the repo and
  grepping.
- Persona: **Priya Nair** (staff platform engineer; evaluator + hands-on adopter).

### J-DOCS-002 — A developer's coding agent pulls grounded Overdrive docs via MCP

> **When** I'm building against Overdrive with an AI coding agent in my
> terminal, **I want** my agent to pull authoritative, current Overdrive docs
> as context via an MCP endpoint, **so I** get correct answers grounded in the
> real docs instead of hallucinations.

- **Functional**: agent produces code grounded in real, current docs.
- **Emotional**: stop babysitting the agent's infra output.
- **Social**: ship working code without becoming the team's Overdrive expert.
- **Four forces** — Push: agents hallucinate plausible-but-wrong infra APIs.
  Pull: the agent grounds itself in the actual docs and the code runs. Anxiety:
  "the MCP endpoint will be a pain to configure / will silently fail." Habit:
  copy-paste from a blog post or trust the agent's stale training data.
- Persona: **Maya Okonkwo** (app developer). The **coding agent is a secondary
  actor**; Maya (the human running it) is the persona.

### J-DOCS-003 — The docs maintainer prioritises from analytics evidence

> **When** I decide what docs to write next, **I want** evidence of what real
> users (humans searching, agents querying via MCP) actually ask — queries,
> frequency, whether they found an answer, the follow-up trajectory — **so I**
> prioritise by real demand and close the highest-impact gaps first.

- **Functional**: prioritise docs work by real demand, not guesswork.
- **Emotional**: confidence that docs effort is well-targeted.
- **Social**: prove the docs platform is improving the evaluator/agent experience.
- **Four forces** — Push: guessing what to document; noisy support channels.
  Pull: a legible signal of exactly what's asked and what returned zero results.
  Anxiety: "the telemetry will be noise / will break the answer path." Habit:
  prioritise by gut or by whoever shouts loudest.
- Persona: **Diego** (docs maintainer / devrel — tertiary). This is the
  strategic differentiator: agent tool-call telemetry is task-contextual and
  trajectory-bearing, so it's a richer prioritisation signal than a chat widget.

### Opportunity scoring (relative, discovery-grade)

| Job | Importance | Current satisfaction | Opportunity | Priority |
|---|---|---|---|---|
| J-DOCS-001 | High (gates adoption) | None (no site) | **Highest** | Slices 01–03, 07, 08 |
| J-DOCS-002 | High (strategic) | None (agents hallucinate) | **High** | Slices 04–05 |
| J-DOCS-003 | Medium (compounding) | None | Medium (depends on 002) | Slice 06 |

J-DOCS-001 is the underserved-and-important job that the walking skeleton and
search slices serve first. J-DOCS-002 is the strategic differentiator.
J-DOCS-003 compounds on top of 002.

---

## Wave: DISCUSS / [HOW] Journey (Two Arcs)

Full detail in `docs/product/journeys/docs-platform.yaml`. Summary of the two
comprehensive emotional arcs (both upward):

### Arc A — Priya (human, browser): Skeptical → Confident

`Land (skeptical) → Search (curious→engaged) → Read concept→how-to (reassured)
→ Decide/proceed (confident)`. Peak tension: the search query — does it return
a real answer or fluff? Relief: the concept page actually explains it AND
links to the how-to (no dead end).

### Arc B — Maya (developer, via agent over MCP): Wary → Trusting

`Configure /mcp once (wary) → agent search_docs (surprised it queried real
docs) → agent get_doc clean markdown (relieved) → agent ships code that runs
(trusting)`. Peak tension: will the agent hallucinate or ground itself? Relief:
the cited URL is real and the spec validates first try. The tool call is
logged (J-DOCS-003) invisibly to Maya.

### Shared-artifact registry (integration-critical)

| Artifact | Source of truth | Consumers | Risk |
|---|---|---|---|
| `docs_content_index` | build-time `source.getPages()` index | search dialog (Arc A), MCP `search_docs` (Arc B), llms.txt/llms-full.txt | **HIGH** — the strategic ONE-INDEX invariant: three+ consumers, one index; must never diverge |
| `page_llm_text` | `getLLMText(page)` primitive | MCP `get_doc`, per-page `.md`, llms-full.txt | **HIGH** — agent must get clean markdown identical to the `.md` export |
| `tool_call_log_record` | MCP route logging wrapper | J-DOCS-003 analytics (maintainer) | MEDIUM — best-effort; must NEVER block/fail the tool response |
| `baseOptions_shell` | one `baseOptions()` nav shell | landing (`/`), docs (`/docs`), blog (`/blog`) | MEDIUM — nav drift across surfaces is an integration failure |

Integration checkpoint: any slice that adds a consumer of `docs_content_index`
(search 03, MCP 05) or `page_llm_text` (export 04, MCP 05) MUST reuse the
single build-time source, never build a parallel index.

---

## Wave: DISCUSS / [HOW] Story Map + Walking Skeleton

### Backbone (chronological user activities, both arcs interleaved)

```
Deploy the     Author &       Find an        Expose to      Ground an      Learn from     Read the       Enter the
site           browse docs    answer         agents         agent          agents         blog           front door
-----------    -----------    -----------    -----------    -----------    -----------    -----------    -----------
01 skeleton    02 content     03 search      04 llms        05 MCP         06 analytics   07 blog        08 landing
   deploy         + nav          (Orama)        export        server         loop
   [SKELETON]
```

### Walking skeleton (the line across all activities, thinnest end-to-end)

Slice 01 — the minimal site live at a real URL — IS the walking skeleton. It is
deliberately NOT a full backbone traversal; the feature's skeleton is "the
deploy pipeline works and serves one page," because that is the fatal
assumption (OpenNext-on-Workers serving Fumadocs at all). Every later slice
layers a backbone activity onto the proven skeleton.

### Priority rationale (outcome impact + dependencies + riskiest-assumption)

1. **Slice 01 (walking skeleton, P1)** — validates the deploy pipeline, the
   single riskiest assumption. Nothing else is real until a URL serves a page.
2. **Slices 02→03 (P1, J-DOCS-001)** — the highest-opportunity job (adoption
   gate). Content then search = time-to-first-answer collapses. 03 depends on 02.
3. **Slices 04→05 (P2, J-DOCS-002, strategic)** — the differentiator. 04
   (export) de-risks 05 (MCP) by proving the `getLLMText` primitive first; 05
   reuses the slice-03 index + slice-04 primitive.
4. **Slice 06 (P2, J-DOCS-003)** — the analytics thesis; compounds on 05.
5. **Slice 07 (P3, J-DOCS-001)** — blog; joins the one index for free.
6. **Slice 08 (P3, J-DOCS-001)** — landing; best last so the home links to
   real docs (02) and blog (07).

Slicing is **by user outcome, not technical layer** — each slice touches the
build/deploy/serve stack end-to-end and ships a behaviour a user can verify.

---

## Wave: DISCUSS / [HOW] System Constraints (cross-cutting)

These apply to every story; not repeated per story.

- **C-1 Stack is decided**: Fumadocs + Next.js (App Router/RSC) + OpenNext on
  Cloudflare Workers. Not open for re-litigation.
- **C-2 Node runtime everywhere**: never `export const runtime = 'edge'`
  (OpenNext manages the runtime). Applies to every route handler.
- **C-3 No runtime `fs` for content**: MDX is compiled into the bundle at build
  time; the Worker never reads `content/` at request time.
- **C-4 One index, multiple consumers**: search, MCP, and llms export all
  consume the SAME build-time `source` index. No parallel indexes.
- **C-5 Greenfield TS subtree**: lives at `website/` (decided 2026-05-30) OUTSIDE
  `crates/`. EXEMPT from Rust crate-class / dst-lint / nextest / cargo-mutants
  gates. Its quality gates are: typecheck, lint, build, and a deploy smoke test.
- **C-6 No aspirational docs**: docs describe only implemented platform
  behaviour (consistent with the repo-wide "no aspirational docs" rule).
- **C-7 Analytics is best-effort**: the tool-call logging path must never block
  or fail an MCP tool response.
- **C-8 Bundle ceiling**: Worker compressed bundle ≤3 MiB Free / 10 MiB Paid;
  the in-Worker Orama index shares the 128 MB isolate ceiling — fine for the
  current corpus, revisit only if it grows large.

---

## Wave: DISCUSS / [HOW] User Stories (LeanUX, with embedded AC)

Every story traces to a `job_id`. Every story has an Elevator Pitch
(Before / After: run `{entry point}` → sees `{observable output}` / Decision
enabled). No `@infrastructure`-only story exists; slice 01's value is a real URL.

Domain data uses real personas (Priya Nair, Maya Okonkwo, Diego) and realistic
queries/URLs, never `user123`.

### US-01: Skeleton docs site live at a real URL

**job_id**: J-DOCS-001

#### Elevator Pitch

- **Before**: There is no overdrive.sh — Priya has nothing to evaluate but the
  Rust source and a static index.html.
- **After**: Priya opens `https://<deployed-url>/docs` → sees a rendered
  Overdrive docs page with the shared nav shell.
- **Decision enabled**: Priya decides the docs site is real and worth exploring
  (and the team decides the chosen stack actually deploys).

#### Problem

Priya Nair is a staff platform engineer evaluating Overdrive. There is no docs
site; she would have to reverse-engineer the platform from source. The team
also doesn't yet know whether Fumadocs-on-Next-via-OpenNext deploys to Workers
at all — the riskiest assumption.

#### Who

- Priya Nair | first visit to overdrive.sh | wants proof the site exists and the stack ships.

#### Solution

A minimal Fumadocs+Next.js site with one docs page and the shared
`baseOptions()` shell, deployed to Cloudflare Workers via OpenNext, live at a
real URL. Subtree at `website/`.

#### Domain Examples

1. **Happy path** — Priya opens `https://overdrive-docs.workers.dev/docs`, sees
   "Overdrive Documentation" rendered with the nav shell. Page returns 200.
2. **Edge case** — Cold Worker: first request after idle still returns the
   rendered page (no blank/error), just with a cold-start latency.
3. **Error/boundary** — A bad path `/docs/does-not-exist` returns a proper
   Fumadocs 404 page, not a Worker error.

#### UAT Scenarios (BDD)

```gherkin
Scenario: The skeleton docs page is reachable at a real URL
  Given the docs site has been deployed to Cloudflare Workers via OpenNext
  When Priya opens the deployed /docs URL in her browser
  Then she sees the rendered docs page with the shared navigation shell
  And the HTTP response status is 200

Scenario: The deploy smoke test confirms the page serves
  Given a fresh deploy of the skeleton site
  When the smoke test issues an HTTP GET to the deployed /docs URL
  Then the response is 200 and the body contains the docs page heading

Scenario: An unknown docs path returns a proper not-found page
  Given the skeleton site is deployed
  When Priya requests a docs path that does not exist
  Then she sees a Fumadocs 404 page, not a raw Worker error
```

#### Acceptance Criteria

- [ ] `GET /docs` on the deployed URL returns 200 with the rendered page + nav shell.
- [ ] The deploy smoke test (HTTP GET on the deployed URL) passes in the slice pipeline.
- [ ] An unknown docs path returns a Fumadocs 404, not a Worker error.
- [ ] No route uses `runtime = 'edge'` (C-2).

#### Outcome KPIs

- **Who**: anonymous first-time visitors. **Does what**: successfully load the
  deployed docs URL. **By how much**: 99%+ of requests return 200 (excluding
  cold-start latency). **Measured by**: Worker request logs / smoke test.
  **Baseline**: 0 (no site exists).

#### Technical Notes

- Scaffold: `npm create cloudflare@latest -- <name> --framework=next --platform=workers`.
- `createMDX()` from `fumadocs-mdx/next`; catch-all `app/docs/[[...slug]]/page.tsx`.
- Constraints C-2, C-3, C-5 apply. No ISR cache binding (SSG).

---

### US-02: Browse real Overdrive docs with working navigation

**job_id**: J-DOCS-001

#### Elevator Pitch

- **Before**: The site has one placeholder page; Priya can't actually learn anything.
- **After**: Priya browses from a concept page to a how-to via the sidebar and
  an in-page link → sees real Overdrive content with sidebar + TOC, no dead ends.
- **Decision enabled**: Priya decides she can learn Overdrive from the docs
  without reading source.

#### Problem

Priya needs to understand the intent/observation boundary and how to submit a
job. A single placeholder page can't answer that; she needs a real, navigable
docs tree.

#### Who

- Priya Nair | browsing to understand a concept and find the matching how-to | wants no dead ends.

#### Solution

A starter set of real MDX docs (concepts + at least one how-to) with `meta.json`
nav ordering, `DocsLayout` sidebar + TOC, and resolving cross-page links.

#### Domain Examples

1. **Happy path** — Priya reads `/docs/concepts/intent-observation-boundary`,
   follows its link to `/docs/how-to/submit-a-job`, lands on the how-to.
2. **Edge case** — A deeply nested page (`/docs/concepts/reconcilers/lifecycle`)
   appears in the right sidebar position per `meta.json`.
3. **Error/boundary** — A page authored with a broken internal link is caught
   (the link target must exist); no published dead ends.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Priya browses a concept page with sidebar and TOC
  Given the docs tree contains a concepts section and a how-to section
  When Priya opens the intent/observation boundary concept page
  Then she sees the page content with the navigation sidebar and a table of contents

Scenario: Priya follows a concept into its how-to with no dead end
  Given Priya is reading the intent/observation boundary concept page
  When she clicks the in-page link to the submit-a-job how-to
  Then she lands on the submit-a-job how-to page

Scenario: The docs describe only implemented behaviour
  Given the starter docs set is published
  When a reviewer reads each page
  Then no page describes platform behaviour that does not exist yet
```

#### Acceptance Criteria

- [ ] A concept page renders with sidebar + TOC.
- [ ] A sidebar link and an in-page link both navigate to the linked how-to.
- [ ] No starter page is a stub or describes unimplemented behaviour (C-6).

#### Outcome KPIs

- **Who**: evaluators browsing docs. **Does what**: navigate from a concept to
  its how-to without leaving the site. **By how much**: 0 dead-end links in the
  starter set. **Measured by**: link-check in the build + manual review.
  **Baseline**: n/a (no content before).

#### Technical Notes

- MDX in `content/docs/`; `meta.json` ordering; Fumadocs MDX components available.
- Depends on US-01 (shell + `lib/source.ts`).

---

### US-03: Find a docs answer fast via search

**job_id**: J-DOCS-001

#### Elevator Pitch

- **Before**: Priya must browse the tree manually to find anything.
- **After**: Priya presses Cmd+K, types "intent observation boundary" → sees a
  ranked result whose top hit is the relevant concept page, and clicks through.
- **Decision enabled**: Priya decides she can get any answer in seconds, which
  cements "this is a serious, well-documented platform."

#### Problem

Browsing is slow for a time-boxed evaluator. Priya wants to type a technical
term and get the right page instantly — and she fears search will return
marketing fluff.

#### Who

- Priya Nair | has a specific technical query | wants a relevant result fast, not fluff.

#### Solution

Orama search over the build-time index: `/api/search` route handler
(`export const { GET } = createFromSource(source)`) + the default Cmd+K dialog.
First consumer of the ONE index (C-4).

#### Domain Examples

1. **Happy path** — Priya types "intent observation boundary"; top result is
   the concept page; she clicks through.
2. **Edge case** — Partial query "reconcil" returns reconciler-related pages
   (prefix/tokenisation works).
3. **Error/boundary** — A nonsense query "qwertyzxcv" returns an empty state,
   not an error.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Search returns a relevant result for a technical query
  Given the docs corpus is indexed by Orama at build time
  When Priya opens search with Cmd+K and types "intent observation boundary"
  Then she sees at least one ranked result
  And the top result is the relevant concept page

Scenario: Clicking a search result navigates to the page
  Given Priya has a ranked search result for her query
  When she clicks the top result
  Then she lands on that docs page

Scenario: An answerable query never returns empty
  Given a term that appears in the published docs
  When Priya searches for that term
  Then the result list is not empty
```

#### Acceptance Criteria

- [ ] Cmd+K opens the dialog; a known technical term returns ≥1 ranked result
      with the relevant page as top hit.
- [ ] Clicking a result navigates to that page.
- [ ] An answerable term does not return empty; a nonsense term returns an
      empty state (not an error).
- [ ] The search index is the same build-time `source` index (C-4); no parallel index.

#### Outcome KPIs

- **Who**: evaluators using search. **Does what**: search then click through to
  a result. **By how much**: search→result-click rate ≥ 50% of searches; median
  time-to-first-answer < 15s from query to landing on the answer page.
  **Measured by**: search-event logging (query → click) where available; manual
  timing for the baseline. **Baseline**: n/a (no search before).

#### Technical Notes

- `app/api/search/route.ts`; in-Worker Orama; Node runtime (C-2); 128 MB
  ceiling fine for current corpus (C-8). Depends on US-02.

---

### US-04: Export docs as clean LLM-readable text

**job_id**: J-DOCS-002

#### Elevator Pitch

- **Before**: There's no machine-readable surface; an agent can only scrape HTML.
- **After**: Maya's agent (or `curl`) fetches `https://overdrive.sh/llms-full.txt`
  or `/docs/how-to/submit-a-job.md` → gets clean title + processed markdown, no
  HTML chrome.
- **Decision enabled**: Maya (or any agent tooling) decides Overdrive's docs are
  consumable as grounded context today, even before the bespoke MCP server.

#### Problem

Maya's agent needs clean, current docs as context. Scraping rendered HTML gives
it marketing chrome and noise. It needs a clean text surface.

#### Who

- Maya Okonkwo (via her agent / `mcpdoc`) | wants the corpus as clean markdown | not HTML soup.

#### Solution

`llms.txt`, `llms-full.txt`, and per-page `.md` via the `getLLMText` primitive
over the build-time source (`includeProcessedMarkdown: true`). Second/third
consumer of the one index.

#### Domain Examples

1. **Happy path** — Agent fetches `/llms-full.txt`, receives concatenated clean
   markdown of all docs pages.
2. **Edge case** — Agent fetches `/docs/concepts/intent-observation-boundary.md`
   and gets that single page's clean markdown.
3. **Error/boundary** — A `.md` request for a nonexistent page returns 404, not
   a fabricated page.

#### UAT Scenarios (BDD)

```gherkin
Scenario: The full corpus is available as clean markdown
  Given the docs source has processed markdown enabled
  When an agent fetches /llms-full.txt
  Then it receives clean concatenated markdown with no rendered HTML chrome

Scenario: A single page is available as clean markdown
  Given a published docs page
  When an agent appends .md to that page's URL and fetches it
  Then it receives that page's title and processed markdown

Scenario: llms.txt indexes the corpus
  Given the docs corpus is built
  When an agent fetches /llms.txt
  Then it receives an index of the doc URLs
```

#### Acceptance Criteria

- [ ] `GET /llms.txt` returns an index of doc URLs.
- [ ] `GET /llms-full.txt` returns clean concatenated markdown (no HTML chrome).
- [ ] `GET /docs/<page>.md` returns that page's clean title + processed markdown.
- [ ] A `.md` request for a nonexistent page returns 404, not a fabricated page.

#### Outcome KPIs

- **Who**: agent tooling consuming the corpus. **Does what**: fetch clean
  markdown for grounding. **By how much**: 100% of published docs pages have a
  reachable clean `.md` and appear in `llms.txt`. **Measured by**: build-time
  assertion (every page → one `.md` + one llms.txt entry). **Baseline**: 0.

#### Technical Notes

- `getLLMText()` + `getText('processed')`; `isMarkdownPreferred()` accept-header
  negotiation. Same build-time source (C-4). Depends on US-02.

---

### US-05: Agent queries docs via the MCP endpoint and grounds its answer

**job_id**: J-DOCS-002

#### Elevator Pitch

- **Before**: Maya's agent hallucinates Overdrive's CLI verbs and reconciler shapes.
- **After**: Maya adds `https://overdrive.sh/mcp` to her agent once; the agent
  calls `search_docs('write a job spec')` then `get_doc(<url>)` → returns clean
  grounded markdown → emits a job spec that validates first try.
- **Decision enabled**: Maya decides to trust her agent on Overdrive specifics
  and stops manually correcting its infra output.

#### Problem

Maya's agent confidently invents Overdrive APIs that don't exist. She wastes
time correcting it. She wants the agent to ground itself in the real docs.

#### Who

- Maya Okonkwo | configures `/mcp` once | wants her agent to ground on real docs autonomously.

#### Solution

A stateless MCP route handler at `/mcp` (Streamable HTTP, Node runtime)
exposing `search_docs(query)` (the slice-03 Orama index) and `get_doc(url)`
(the slice-04 `getLLMText`). Fourth consumer of the one index.

#### Domain Examples

1. **Happy path** — Agent calls `search_docs('submit a job')`, gets the how-to
   URL, calls `get_doc(<url>)`, grounds a valid job spec.
2. **Edge case** — Agent calls `search_docs('nonexistent feature xyz')`, gets
   zero results; the agent reports it can't find docs rather than fabricating.
3. **Error/boundary** — Agent calls `get_doc(<nonexistent url>)`; the tool
   returns an honest not-found, never a fabricated page.

#### UAT Scenarios (BDD)

```gherkin
Scenario: An agent grounds a job spec via the MCP tools
  Given Maya has configured the overdrive.sh MCP endpoint in her agent
  When her agent calls search_docs for "submit a job" and then get_doc on the top result URL
  Then get_doc returns clean markdown for the submit-a-job how-to
  And the agent's resulting job spec uses real CLI verbs and shapes

Scenario: search_docs returns ranked, relevant results
  Given the MCP endpoint is live and reusing the build-time docs index
  When an agent calls search_docs with "submit a job"
  Then it receives ranked results whose top hit is the relevant how-to URL

Scenario: get_doc on a missing URL is honest, not fabricated
  Given the MCP endpoint is live
  When an agent calls get_doc with a URL that does not resolve to a docs page
  Then the tool returns an honest not-found result, not invented page content
```

#### Acceptance Criteria

- [ ] An MCP client connects to `/mcp` and lists `search_docs` and `get_doc`.
- [ ] `search_docs('submit a job')` returns ranked results with the relevant
      how-to URL as top hit.
- [ ] `get_doc(<that url>)` returns clean markdown identical to the slice-04
      `.md` for the same page.
- [ ] `get_doc(<nonexistent url>)` returns an honest not-found, not a fabricated page.
- [ ] The endpoint is stateless, Node runtime, never edge (C-2).

#### Outcome KPIs

- **Who**: developers whose agents use the endpoint. **Does what**: agent
  produces grounded Overdrive code that runs. **By how much**: MCP tool-call
  volume (adoption signal) growing month-over-month; re-query rate (agent
  re-searching the same task after get_doc) < 30% — low re-query = the agent
  found the answer (the "answer quality" proxy). **Measured by**: the slice-06
  tool-call log. **Baseline**: 0 (no endpoint).

#### Technical Notes

- `app/mcp/route.ts`; Streamable HTTP; stateless `createMcpHandler()`-shape;
  zod schemas; reuses slice-03 index + slice-04 primitive. Depends on US-03, US-04.

---

### US-06: Maintainer sees agent demand from logged MCP tool calls

**job_id**: J-DOCS-003

#### Elevator Pitch

- **Before**: Diego guesses what docs to write next from a noisy support channel.
- **After**: Diego queries the tool-call log → sees the top `search_docs`
  queries and which returned `result_count = 0` → names the highest-demand gap.
- **Decision enabled**: Diego decides which docs page to write next based on
  real agent demand, not guesswork.

#### Problem

Diego (docs maintainer) doesn't know what users actually ask. He guesses, or
reacts to whoever shouts loudest. He wants evidence — especially the
zero-result queries that reveal coverage gaps.

#### Who

- Diego | docs maintainer / devrel | wants real demand evidence to prioritise docs.

#### Solution

A best-effort logging wrapper around the slice-05 MCP handlers recording
`{tool, query, ts, result_count}` to a Cloudflare binding (Analytics Engine vs
D1 — a DESIGN decision), plus a way for Diego to read the aggregate.

#### Domain Examples

1. **Happy path** — After a week of agent traffic, Diego reads the log and sees
   "configure mtls" was the top zero-result query; he writes that page next.
2. **Edge case** — A burst of identical queries from one agent session is
   visible as repeated rows (frequency signal intact).
3. **Error/boundary** — The logging binding is down; the agent's `search_docs`
   response is unaffected (best-effort, C-7).

#### UAT Scenarios (BDD)

```gherkin
Scenario: Tool calls are captured for the maintainer
  Given the MCP endpoint is live with analytics logging enabled
  When an agent makes search_docs and get_doc calls
  Then the maintainer can read rows recording tool, query, timestamp, and result count

Scenario: Zero-result queries are captured as coverage-gap signal
  Given an agent searches for a topic the docs do not cover
  When the maintainer reads the log
  Then the query appears with a result count of zero

Scenario: Logging never breaks the agent's answer
  Given the analytics binding is unavailable
  When an agent calls search_docs
  Then the agent still receives its normal tool response
```

#### Acceptance Criteria

- [ ] After agent tool calls, the maintainer reads `{tool, query, ts,
      result_count}` rows from the binding.
- [ ] A zero-result `search_docs` produces a row with `result_count = 0`.
- [ ] Forcing a logging failure does not change the tool response (C-7).

#### Outcome KPIs

- **Who**: the docs maintainer. **Does what**: prioritise the next docs page
  from logged demand. **By how much**: ≥ 1 docs page per month authored in
  direct response to a top zero-result query; zero-result-query rate trending
  down over time (coverage improving). **Measured by**: the tool-call log +
  the docs changelog citing the query that motivated each page. **Baseline**: 0.

#### Technical Notes

- Logging wrapper around slice-05 handlers; binding = DESIGN decision
  (Analytics Engine vs D1); best-effort (C-7). Depends on US-05.

---

### US-07: Read blog posts that are also searchable + MCP-discoverable

**job_id**: J-DOCS-001

#### Elevator Pitch

- **Before**: There's no place for Overdrive narrative/announcement content.
- **After**: Priya opens `/blog` → sees a post list, reads a post; a term unique
  to that post returns it in search AND in MCP `search_docs`.
- **Decision enabled**: Priya (and agents) discover narrative content through
  the same search/MCP surface, deepening the "well-documented" impression.

#### Problem

Concepts and how-tos don't cover announcements, design rationale, or narrative.
A blog fills that — and it should join the one index so it's discoverable for free.

#### Who

- Priya Nair (and Maya's agent) | wants narrative content discoverable via the same surfaces.

#### Solution

A second `defineCollections({ type: 'doc' })` blog collection with hand-rolled
list/post pages (no turnkey Fumadocs blog layout), feeding the same build-time index.

#### Domain Examples

1. **Happy path** — Priya opens `/blog`, sees posts with titles + dates, opens
   "Why Overdrive collapses the K8s stack," reads it.
2. **Edge case** — A post with a unique term "kTLS-offload" is found by search
   (slice 03) and by MCP `search_docs` (slice 05).
3. **Error/boundary** — A draft post (frontmatter `draft: true`, if used) does
   not appear in the list or the index.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A visitor reads a blog post under the shared shell
  Given the blog collection has at least one published post
  When Priya opens /blog and then a post
  Then she sees the post list with titles and dates, and the post body, under the shared nav shell

Scenario: Blog content joins the one index for search and MCP
  Given a published blog post contains a unique term
  When Priya searches for that term and an agent calls search_docs for it
  Then both return the blog post
```

#### Acceptance Criteria

- [ ] `GET /blog` lists posts (title, date) under the shared `baseOptions()` shell.
- [ ] `GET /blog/<slug>` renders the post body.
- [ ] A term unique to a post returns it in search (slice 03) AND MCP
      `search_docs` (slice 05) — proving it joined the one index (C-4).

#### Outcome KPIs

- **Who**: visitors and agents. **Does what**: discover blog content via search/MCP.
  **By how much**: 100% of published posts are search- and MCP-discoverable.
  **Measured by**: build assertion (each post appears in the index). **Baseline**: 0.

#### Technical Notes

- `content/blog/` flat collection; `app/(home)/blog/page.tsx` +
  `app/(home)/blog/[slug]/page.tsx` (documented Next components); shared shell.
  Depends on US-01; indexing benefits use US-03/04/05.

---

### US-08: Land on the marketing home and navigate into docs/blog

**job_id**: J-DOCS-001

#### Elevator Pitch

- **Before**: The site has no front door; visitors land directly in /docs with
  no value-prop context.
- **After**: Priya opens `https://overdrive.sh/` → sees the value prop (from
  index.html) in a `HomeLayout`, and navigates into /docs and /blog via the shared shell.
- **Decision enabled**: Priya decides, in the first seconds, whether to dig in —
  and the nav gives her one coherent path across all surfaces.

#### Problem

There's no marketing front door tying the surfaces together. The existing
`index.html` has the message but isn't part of the platform shell.

#### Who

- Priya Nair | first lands on `/` | wants a clear value prop and a path into docs/blog.

#### Solution

`/` using `HomeLayout` with marketing content adapted from the existing
`index.html` (placeholder, not a redesign), sharing the `baseOptions()` shell.

#### Domain Examples

1. **Happy path** — Priya opens `/`, reads the "one Rust binary, kernel-native
   dataplane" value prop, clicks into `/docs`.
2. **Edge case** — From `/`, she clicks into `/blog` and the same nav shell
   persists (no drift).
3. **Error/boundary** — The home's nav shell is byte-for-byte the same
   `baseOptions()` shell as docs and blog (no divergent nav).

#### UAT Scenarios (BDD)

```gherkin
Scenario: The landing page presents the value prop and a path into docs
  Given the landing page is deployed using HomeLayout
  When Priya opens the site root
  Then she sees the Overdrive value proposition from index.html
  And she can navigate into /docs from the nav shell

Scenario: The nav shell is consistent across all three surfaces
  Given the landing, docs, and blog surfaces are deployed
  When Priya navigates between /, /docs, and /blog
  Then the same baseOptions navigation shell is present on each
```

#### Acceptance Criteria

- [ ] `GET /` returns `HomeLayout` with the index.html value prop.
- [ ] The nav shell links to `/docs` and `/blog`, both navigate correctly.
- [ ] The `/` nav shell is the same `baseOptions()` shell as docs and blog (no drift).

#### Outcome KPIs

- **Who**: first-time visitors landing on `/`. **Does what**: navigate from `/`
  into `/docs` or `/blog`. **By how much**: ≥ 40% of `/` sessions proceed into
  docs or blog. **Measured by**: navigation/click logging where available.
  **Baseline**: n/a (no home).

#### Technical Notes

- `HomeLayout` at `/`; content from repo-root `index.html`; `unoptimized`
  images or simple loader; shared shell. Best last (links to US-02 docs, US-07 blog).

---

## Wave: DISCUSS / [REF] Outcome KPIs (rollup)

### Objective

By the end of the docs-platform feature, overdrive.sh answers evaluators'
questions in seconds and grounds coding agents in current docs — and the
maintainer can prove it by watching what users actually ask.

### KPI table

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | First-time visitors | load the deployed docs URL | 99%+ requests 200 | 0 | Worker logs / smoke test | Leading (secondary) |
| 2 | Evaluators | search→click to an answer | ≥50% click-through; TTFA <15s | n/a | search-event logging | Leading |
| 3 | Agent tooling | fetch clean `.md`/llms for grounding | 100% pages exported | 0 | build assertion | Leading (secondary) |
| 4 | Developers (via agents) | agent ships grounded code | MCP volume ↑ MoM; re-query <30% | 0 | tool-call log | Leading |
| 5 | Docs maintainer | author docs from logged demand | ≥1 page/mo from top zero-result query; zero-result rate ↓ | 0 | tool-call log + changelog | Leading |
| 6 | First-time visitors | navigate `/` → docs/blog | ≥40% proceed | n/a | nav logging | Leading (secondary) |

### North star

**MCP tool-call volume with low re-query rate** — the strategic thesis made
measurable: agents adopting the endpoint (volume up) AND finding answers on the
first/second call (re-query down) is the signal that Overdrive is a first-class
agent context.

### Guardrails (must NOT degrade)

- Page load success rate (KPI 1) must not drop when features are added.
- Analytics logging must never block/fail an MCP tool response (C-7).
- Search/MCP must reuse ONE index — no divergence between human and agent answers (C-4).

### Measurement note (instrumentation dependency)

KPIs 2, 4, 5, 6 depend on event logging. KPIs 4/5 are served by the slice-06
tool-call log. KPIs 2/6 (browser-side click/nav logging) are **not currently
specified as a slice** — see Deferral D-3.

---

## Wave: DISCUSS / [REF] DoR Validation (9-item hard gate)

Validated per story (US-01…US-08). Summary table; all stories share the same
structure (real personas, 3 examples, 3 scenarios, embedded AC, KPIs).

| DoR Item | US-01 | US-02 | US-03 | US-04 | US-05 | US-06 | US-07 | US-08 |
|---|---|---|---|---|---|---|---|---|
| 1 Problem clear, domain language | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 2 Persona with characteristics | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 3 ≥3 domain examples, real data | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 4 UAT G/W/T (3–7 scenarios) | PASS (3) | PASS (3) | PASS (3) | PASS (3) | PASS (3) | PASS (3) | PASS (2)* | PASS (2)* |
| 5 AC derived from UAT | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 6 Right-sized (1–3 days, 3–7 scen) | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 7 Technical notes / constraints | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 8 Dependencies resolved/tracked | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 9 Outcome KPIs w/ measurable targets | PASS | PASS | PASS | PARTIAL† | PASS | PASS | PASS | PARTIAL‡ |

\* US-07 and US-08 carry **2** UAT scenarios each, below the DoR minimum of 3.
This is the one DoR item that could not be fully satisfied as written. Both are
genuinely small (a list+post page; a home page reusing existing copy) — the
scenarios cover happy path + the cross-surface invariant. **Remediation**: add
a third scenario each in DESIGN/DISTILL (US-07: a draft/unpublished post is
excluded; US-08: cold-load of `/` returns the value prop). Flagged, not blocking
at slice granularity since each is < 1 day.

† US-04 KPI ("100% pages exported") is a coverage metric, not a behaviour-change
metric — acceptable as a leading-secondary indicator but weaker than an outcome
KPI. **Remediation**: pair with KPI 4 (the behaviour change is downstream in
US-05's MCP grounding).

‡ US-08 KPI (≥40% proceed) depends on nav logging not yet specified as a slice
(Deferral D-3).

### DoR overall: PASS with two tracked remediations

The 9-item gate passes for all 8 stories with the explicit exceptions above:
(a) US-07/US-08 each have 2 (not 3+) UAT scenarios — small stories, third
scenario to be added downstream; (b) browser-side KPI instrumentation (KPIs
2/6) is unspecified — Deferral D-3. No story is BLOCKED. Per the lean review
posture, per-wave peer review is skipped (consolidated review fires at end of
DISTILL); these two items did not surface ambiguity warranting a peer review.

---

## Wave: DISCUSS / [REF] Wave Decisions

### Scope Assessment: SPLIT — 8 slices, 1 module (website/), estimated ~8 days

Oversized (4 signals); split into 8 thin end-to-end slices, walking skeleton =
slice 01. User pre-confirmed the split + arc via the wizard decisions.

### Decisions

- **D-DEC-1 Subtree location** (DECIDED 2026-05-30 — `website/`, chosen over
  `apps/docs/`/`www/`/`site/` and over repo-root `docs/` which collides with the
  existing whitepaper/ADR/nWave tree): `website/` at repo root, OUTSIDE
  `crates/`. EXEMPT from Rust crate-class / dst-lint / nextest / cargo-mutants
  gates (C-5). Quality gates: typecheck, lint, build, deploy smoke test. *This
  is a proposal for DESIGN to ratify; `www/` is the alternative the user named.*
- **D-DEC-2 Stack locked** (given): Fumadocs + Next.js App Router/RSC + OpenNext
  on Cloudflare Workers. Not re-litigated.
- **D-DEC-3 One index, multiple consumers** (architectural invariant, C-4):
  search, MCP, llms export consume one build-time `source` index.
- **D-DEC-4 Discovery skipped** (given): problem validated (the project needs
  docs). Recorded as accepted, not a risk.
- **D-DEC-5 JTBD bootstrapped**: J-DOCS-001/002/003 are NEW jobs (a different
  KIND than J-PLAT-*/J-OPS-*). Appended to `jobs.yaml` with changelog.

### Risks

- **R-1** OpenNext-on-Workers serving Fumadocs is unproven in THIS repo →
  mitigated by making it the walking skeleton (slice 01).
- **R-2** In-Worker Orama index could approach the 128 MB isolate ceiling if the
  corpus grows large → guardrail C-8; external search (Orama Cloud/Algolia) is
  the documented escape hatch, deferred (D-1).
- **R-3** Stale docs producing confidently-wrong agent answers (worse than no
  answer) → mitigated by C-6 (no aspirational docs) + the analytics loop
  surfacing mismatches indirectly.

---

## Wave: DISCUSS / [REF] Deferrals — REQUIRE USER APPROVAL (surfaced, NOT acted on)

Per CLAUDE.md deferral discipline: surfaced here for the orchestrator to relay.
**No GitHub issues created. No issue numbers invented.**

- **D-1 External search (Orama Cloud / Algolia)** — in-Worker Orama is the
  slice-03 choice; migrating to external search if the corpus outgrows the
  128 MB ceiling is deferred. Needs user approval before any "wire later" note
  or issue.
- **D-2 Analytics binding choice (Analytics Engine vs D1)** — slice 06 specifies
  WHAT is logged + the best-effort contract; WHICH binding is a DESIGN decision.
  Surfaced as a decision DESIGN must make, not a silent deferral.
- **D-3 Browser-side event instrumentation** (search click-through KPI 2, nav
  KPI 6) — RESOLVED 2026-05-30: use **Cloudflare Web Analytics**
  (https://www.cloudflare.com/web-analytics/) rather than a dedicated
  browser-analytics slice. No 9th slice. DESIGN must confirm whether CF Web
  Analytics' RUM/page-view surface can capture the two funnel events
  (search→result-click, landing→proceed) directly or whether custom-event
  support / a lightweight event beacon is needed to make KPIs 2/6 measurable;
  if custom events are unavailable, KPIs 2/6 are approximated from
  page-view funnels at baseline.
- **D-4 RSS / OG image for the blog** (slice 07 OUT-of-scope) — desirable
  follow-up; deferred.
- **D-5 `fumadocs-openapi` interactive API playground** — available on the
  Next.js/RSC path (the reason Next was chosen over TanStack), but NOT scoped in
  any slice. Plausibly wanted for an API-driven platform; deferred pending user
  direction.
- **D-6 Custom domain wiring** (`overdrive.sh` vs the `workers.dev` URL) —
  slice 01 accepts a `workers.dev` URL for the skeleton; production domain
  wiring deferred.

---

## Wave: DESIGN / [REF] Wave Decisions

**Wave**: DESIGN (Morgan, nw-solution-architect). **Mode**: GUIDE, pass 2 of 2
(decisions locked in pass 1; this pass writes the SSOT). **Date**: 2026-05-30.
**Rigor**: lean / Tier-1. **Outcome Collision Check**: N/A — no
`docs/product/outcomes/registry.yaml` exists.

The authoritative DESIGN prose, C4 diagrams (System Context L1 + Container L2 +
Component L3), full component decomposition, technology stack with versions, and
quality-attribute mapping live in
`docs/product/architecture/brief.md` § `## docs-platform website (overdrive.sh)`.
This feature-delta section is the lean DESIGN record; it references the brief
rather than duplicating it.

**Non-contested choices carried from pass 1** (pinned, not re-litigated):
modular monolith = one Next app / one OpenNext Worker; ports-and-adapters-
equivalent via the `source` / `lib/search.ts` / `lib/get-llm-text.ts` /
`lib/site.ts` seams; Node runtime everywhere (never `runtime = 'edge'`, C-2);
SSG build-time content (no runtime `fs`, C-3); `getLLMText` is ONE primitive
shared by MCP `get_doc`, per-page `.md`, and `llms-full.txt` (`get_doc`
byte-identical to `.md`, US-05 AC); OSS-first (Fumadocs/Next/MCP-SDK MIT, Orama
Apache-2.0); TypeScript/React/Next.js paradigm — NOT written to CLAUDE.md (the
OOP line governs Rust only).

## Wave: DESIGN / [REF] DDD — Design Decisions (verdicts)

| ID | Decision | Verdict | Rationale | ADR |
|---|---|---|---|---|
| D-A | MCP transport = same-Worker Next route handler (`app/mcp/route.ts`, Node) | ACCEPT | one in-process index = strongest C-4 no-divergence guarantee; `createMcpHandler()` allowed inside (latitude) | ADR-0055 |
| D-B | Analytics binding = D1; logging best-effort (`ctx.waitUntil` + catch-swallow) | ACCEPT | top-zero-result = one `SELECT … WHERE result_count=0 GROUP BY query`; C-7 — never alters/delays the response | ADR-0056 |
| D-C | Search = in-Worker Orama now, behind `lib/search.ts` seam; benchmarked migration trigger | ACCEPT | simplest viable for launch corpus; one query path; single-file external swap; trigger >~5k pages OR ~60–70 MB of 128 MB isolate (inference, benchmark) | ADR-0057 |
| D-D | Browser KPIs = page-view funnel approximation (CF Web Analytics) | ACCEPT | KPI-2 (search→click) + KPI-6 (landing→proceed) explicitly approximated; no custom-event beacon; no 9th slice | (this doc) |
| D-E | OpenAPI playground (`fumadocs-openapi`) | OUT OF SCOPE | user non-goal; Next/RSC path keeps it addable later with zero rework — a property, not a promise | (none) |
| D-F | Single `SITE_ORIGIN` config constant (`lib/site.ts`) | ACCEPT | feeds llms.txt absolute URLs, MCP `get_doc` resolution, canonical/OG; skeleton `workers.dev` → prod flips one constant; DNS/binding is DEVOPS | (brief) |
| D-G | `website/` App Router layout; content in `content/docs|blog/` (not repo-root `docs/`) | ACCEPT | repo-root `docs/` holds whitepaper/ADR/nWave tree; C-6 — site ≠ internal-design mirror; one `baseOptions()`; no R2 ISR; `next/image` unoptimized | (brief) |
| D-H | Build-time one-index enforcement assertion (Node step in `website/`, NOT Rust gate) | ACCEPT | every page → reachable `.md` + in `llms.txt` + in search index; blog in same index — C-4 made structural (nWave principle 11/12) | ADR-0058 |

## Wave: DESIGN / [REF] Component Decomposition

One container: the OpenNext Cloudflare Worker (modular monolith). Full table in
brief § Component decomposition. CREATE-NEW glue only: MCP route handler + tool
schemas; D1 logging wrapper; `lib/search.ts` / `lib/get-llm-text.ts` /
`lib/site.ts` seams; blog list/post components; landing content port; the
one-index assertion script. Everything else is library-primitive USE.

| Component | Path (`website/`) | Change |
|---|---|---|
| MDX plugin / source / shell | `next.config.*`, `lib/source.ts`, `lib/layout.shared.tsx` | USE |
| Docs / search API / llms exports | `app/docs/[[...slug]]`, `app/api/search/route.ts`, `app/llms*`, `*.md` | USE (call seams) |
| Search seam | `lib/search.ts` | CREATE-NEW |
| LLM-text seam | `lib/get-llm-text.ts` | CREATE-NEW |
| Site-origin config | `lib/site.ts` | CREATE-NEW |
| Landing | `app/(home)/page.tsx` | CREATE-NEW |
| Blog list + post | `app/(home)/blog/page.tsx`, `app/(home)/blog/[slug]/page.tsx` | CREATE-NEW |
| MCP endpoint + logging wrapper | `app/mcp/route.ts` | CREATE-NEW |
| One-index assertion | `scripts/assert-one-index.ts` | CREATE-NEW |
| Content | `content/docs/`, `content/blog/` | content |

## Wave: DESIGN / [REF] Driving Ports

`GET /` (US-08) · `GET /docs/[[...slug]]` (US-01/02) · `GET /blog`,
`GET /blog/[slug]` (US-07) · `GET /api/search` (US-03) · `POST/GET /mcp`
(US-05/06) · `GET /llms.txt`, `GET /llms-full.txt`, `GET /docs/<page>.md`
(US-04).

## Wave: DESIGN / [REF] Driven Ports + Adapters

| Driven port / seam | Adapter | ADR |
|---|---|---|
| `source` (one build-time index) | `lib/source.ts` over Fumadocs `loader()` | (C-4) |
| `searchIndex(query)` | `lib/search.ts` over in-Worker Orama | ADR-0057 |
| `getLLMText(page)` | `lib/get-llm-text.ts` over `getText('processed')` | (US-05) |
| `SITE_ORIGIN` | `lib/site.ts` | (D-F) |
| analytics sink | D1 `tool_calls` (best-effort) | ADR-0056 |
| RUM funnels | Cloudflare Web Analytics | (D-D) |

## Wave: DESIGN / [REF] Technology Choices (pinned)

Next 15 (latest minor) or 16 (MIT) · Fumadocs v16 + `fumadocs-mdx` (MIT) ·
Orama (Apache-2.0, Fumadocs-bundled) · `@opennextjs/cloudflare` (MIT) · MCP TS
SDK `@modelcontextprotocol/sdk` (MIT) + optional CF Agents `createMcpHandler()`
· `zod` (MIT) · React 19 / TypeScript 5.x. Platform bindings: Cloudflare
Workers (Node runtime, 128 MB isolate, 3/10 MiB bundle), D1, Web Analytics. Full
table in brief § Technology stack.

## Wave: DESIGN / [REF] Reuse Analysis (HARD GATE)

Full table in brief § Reuse Analysis. Summary: 9 library-primitive USE rows
(MDX pipeline, source/one-index, docs layout + nav + TOC, `baseOptions()`,
Orama `/api/search`, llms exports, Cmd+K dialog, MCP transport, OpenNext deploy)
vs 8 CREATE-NEW glue rows (MCP handler + schemas, D1 logging wrapper, the three
`lib/*` seams, blog components, landing content port, one-index assertion). No
CREATE-NEW row re-implements a library primitive; each encodes an application
invariant (C-4, C-7) or our content. No proprietary library dependency.

## Wave: DESIGN / [REF] Open Questions (carried to DEVOPS/DISTILL)

- Custom-domain DNS + Workers binding (`overdrive.sh`) — **DEVOPS**; one
  `SITE_ORIGIN` flip from `workers.dev` (D-F). Not a deferral-with-forward-
  pointer.
- External-search migration trigger numbers (>~5k pages / ~60–70 MB) —
  **inference to be benchmarked** against the real corpus (ADR-0057). A
  measurement task, not a committed limit.
- KPI-2 / KPI-6 — **approximated from page-view funnels** at baseline (D-D); no
  custom-event instrumentation in scope.
- DISTILL remediation (carried from DISCUSS DoR): US-07 add a third UAT scenario
  (draft/unpublished post excluded from list + index); US-08 add a third
  (cold-load of `/` returns the value prop). Both flagged in DISCUSS; not
  blocking at slice granularity.

## Wave: DESIGN / [REF] Changed Assumptions

None. The stack and all eight decisions were locked in pass 1; pass 2 wrote the
SSOT artifacts without altering any DISCUSS assumption, story, or AC. No
`design/upstream-changes.md` was required.

## Wave: DEVOPS / [REF] Wave Decisions

**Wave**: DEVOPS (Apex, nw-platform-architect). **Mode**: lean / Tier-1. **Date**:
2026-05-30. All decisions below were locked by the user before this wave; this
section records them as the SSOT. Per-wave Forge (peer) review SKIPPED — no
novel deploy target beyond the locked stack; consolidated review fires at
DISTILL. Machine artifacts: `environments.yaml` (this feature dir),
`docs/product/kpi-contracts.yaml` (SSOT).

| ID | Decision | Verdict | Rationale |
|---|---|---|---|
| V-1 | Deployment target = Cloudflare Workers (edge), via `@opennextjs/cloudflare` (OpenNext) | LOCKED | the DESIGN container; Node runtime everywhere (C-2), SSG (C-3) |
| V-2 | Container orchestration = none (serverless Workers, no containers) | LOCKED | one Worker = the modular monolith; nothing to orchestrate |
| V-3 | CI/CD platform = GitHub Actions | LOCKED | the repo already uses it; coexists with the Rust pipelines |
| V-4 | New website workflow, path-scoped to `website/**`; supersedes `deploy-pages.yml` | LOCKED | greenfield; must never run Rust gates, never break them |
| V-5 | Observability = Cloudflare-native (Workers Logs + Web Analytics + D1) | LOCKED | no OTel/Datadog/Prometheus/ELK; matches the deploy target |
| V-6 | Deployment strategy = atomic deploy + instant rollback via `wrangler` | LOCKED | no canary/gradual for a docs site |
| V-7 | Continuous learning = none at deploy level (no A/B, no feature flags) | LOCKED | the product MCP tool-call loop (J-DOCS-003) IS the learning signal — product instrumentation, not a deploy experiment framework |
| V-8 | Branching = GitHub Flow (feature branch -> PR -> main) | LOCKED | matches current practice |
| V-9 | Mutation testing = DISABLED for `website/` | LOCKED | TS subtree, C-5-exempt; gates are tsc + lint + build + deploy-smoke + the one-index assertion (ADR-0058). The Rust workspace's per-feature cargo-mutants strategy in CLAUDE.md is UNCHANGED and does not apply here |

### Risks (DEVOPS)

- **DR-1** Two pipelines in one repo could cross-trigger (Rust CI on a docs typo;
  website CI on a Rust change) → mitigated by path-scoping each workflow (V-4).
  Today's `ci.yml` has no path filter; an optional non-breaking `paths-ignore`
  improvement is surfaced (not required) in the Coexistence Matrix below.
- **DR-2** Removing `deploy-pages.yml` before the Cloudflare deploy exists would
  leave the docs with no deploy → mitigated by scheduling removal in DELIVER
  slice 01 ALONGSIDE the working Workers deploy, never before.
- **DR-3** Secrets (`CLOUDFLARE_API_TOKEN`, account id) absent at first deploy →
  concrete DELIVER-slice-01 / DEVOPS-infra prerequisite, stated below; no values
  invented.

## Wave: DEVOPS / [REF] Environment Matrix

Full machine artifact: `docs/feature/docs-platform/environments.yaml`. Summary:

| Environment | Runtime | Trigger | Deploy | Rollback |
|---|---|---|---|---|
| `local-dev` | Node (`next dev`) / local workerd (`wrangler dev`) | manual | none | n/a |
| `preview` | Cloudflare Workers (OpenNext, Node) | PR on `website/**` | `wrangler versions upload` (preview alias) | superseded by next upload |
| `production` | Cloudflare Workers (OpenNext, Node) | push to `main` on `website/**` | `wrangler deploy` (atomic) | `wrangler rollback` (instant) |

`SITE_ORIGIN` is `workers.dev` for the skeleton and flips to `https://overdrive.sh`
in production (one constant, D-F). The website inner loop does NOT use the Rust
Lima VM — Lima governs the Rust workspace only.

## Wave: DEVOPS / [REF] CI/CD Pipeline Outline (NEW website workflow)

A NEW GitHub Actions workflow (lands in DELIVER slice 01) for the `website/`
subtree, **path-filtered to `website/**`** so it never runs on Rust changes and
the Rust gates never run on website changes. Stages (lean — no YAML dump here;
the skeleton lands in DELIVER):

1. **Commit stage (CI, blocking)** — on `pull_request` touching `website/**`:
   `tsc --noEmit` (typecheck) · ESLint (lint) · `opennextjs-cloudflare build`
   (build) · `scripts/assert-one-index.ts` (the build-time one-index assertion,
   ADR-0058 — every page → reachable `.md` + in `llms.txt` + in search index).
2. **Acceptance stage (CI/deploy, blocking)** — preview deploy
   (`wrangler versions upload`) + the **deploy smoke test** (HTTP GET on the
   deployed `/docs`: assert 200 + heading present, US-01 AC).
3. **Production deploy** — on `push` to `main` touching `website/**`:
   `wrangler deploy` (atomic) after the same commit-stage gates, then the
   production smoke test (advisory; failing smoke → manual `wrangler rollback`).

This workflow **supersedes `deploy-pages.yml`**. Local quality gates: a
`website/`-scoped pre-commit (tsc + lint, mirroring the commit stage) MAY be
added in DELIVER if wanted; not required by this design. No SAST/DAST/SBOM
stages — public docs site, no auth surface, no secrets in the bundle (the only
secret is the deploy token, referenced from GH Actions, never committed).

## Wave: DEVOPS / [REF] Monitoring Contracts (KPI → instrument)

SSOT: `docs/product/kpi-contracts.yaml`. Every outcome KPI maps to a
Cloudflare-native instrument:

| KPI | What | Instrument | Note |
|---|---|---|---|
| KPI-1 | 99%+ requests 200 | Workers Logs + deploy smoke test | **guardrail**; alert on success-rate drop |
| KPI-2 | search→click ≥50%, TTFA <15s | Cloudflare Web Analytics page-view funnel | **APPROXIMATED** (no custom-event beacon, D-D) |
| KPI-3 | 100% pages exported | build-time one-index assertion (ADR-0058) | blocking CI gate |
| KPI-4 | MCP volume ↑ MoM, re-query <30% | D1 `tool_calls` log | best-effort (C-7 / ADR-0056) |
| KPI-5 | ≥1 page/mo from top zero-result query; zero-result rate ↓ | D1 `tool_calls` (`SELECT … WHERE result_count=0 GROUP BY query`) + docs changelog | best-effort |
| KPI-6 | `/`→docs/blog ≥40% | Cloudflare Web Analytics page-view funnel | **APPROXIMATED** (D-D) |
| North star | MCP volume + low re-query | derived from D1 `tool_calls` | volume = row count; re-query = repeated search_docs/session |

Honest approximation statement: KPIs 2 and 6 are page-view-funnel proxies, not
instrumented click/nav events — there is no custom-event beacon and no 9th
slice (D-D). The D1 log (KPIs 4/5) is best-effort: a logging failure NEVER
blocks/delays/alters the MCP tool response (C-7, guardrail G-2).

## Wave: DEVOPS / [REF] Deployment Strategy (atomic + instant rollback)

The docs site deploys atomically: `wrangler deploy` builds an immutable Worker
version and re-points the production route at it in one step — there is no
mixed-version window, no canary, no gradual traffic shift (unnecessary for a
stateless SSG docs site). **Rollback contract**: every prior deploy is a
retained immutable Worker version; `wrangler rollback [<version-id>]` re-points
the live route at the previous version instantly, with no rebuild. Trigger for
manual rollback: deploy smoke test failure, a stakeholder-reported broken page,
or a KPI-1 success-rate drop. The D1 `tool_calls` table is additive
best-effort logging with no destructive migration in scope, so there is no data
rollback concern at launch; the schema is created once (a concrete DELIVER /
DEVOPS prerequisite, below).

## Wave: DEVOPS / [REF] Mutation Testing Strategy (DISABLED for website)

Mutation testing is **DISABLED for the `website/` subtree.** Rationale: `website/`
is a greenfield TypeScript/Next.js app, C-5-exempt from the Rust quality gates;
its correctness gates are typecheck + lint + build + deploy smoke test + the
build-time one-index assertion (ADR-0058). The strongest invariant (C-4 one
index) is enforced structurally by the assertion, not by mutation kill-rate. The
Rust workspace's per-feature `cargo-mutants` strategy (in CLAUDE.md
§ Mutation Testing Strategy) is **UNCHANGED and does not apply** to `website/` —
this decision is recorded ONLY in the feature artifacts, never written as a
global/conflicting CLAUDE.md line.

## Wave: DEVOPS / [REF] Observability Stack (Cloudflare-native per signal class)

| Signal class | Tool | Captures |
|---|---|---|
| Logs / request health | Cloudflare Workers Logs / observability | per-request status, path, timing (KPI-1) |
| Browser RUM (funnels) | Cloudflare Web Analytics | page views, navigation funnels (KPIs 2/6, approximated) |
| Product analytics (MCP) | Cloudflare D1 (`tool_calls`) | `{tool, query, ts, result_count}` (KPIs 4/5, north star) |
| Build-time correctness | website CI (one-index assertion) | every page exported + indexed (KPI-3) |

No OpenTelemetry, no Datadog/Prometheus/ELK — the stack is Cloudflare-native,
matching the deploy target and the lean operational posture.

## Wave: DEVOPS / [REF] Branching Strategy (GitHub Flow + CI trigger alignment)

GitHub Flow: feature branch → PR → `main`. The active branch is
`marcus-sa/fumadocs-mcp-search-setup`; default branch `main`. CI trigger
alignment for the website workflow: `pull_request` (commit + acceptance stages,
incl. preview deploy + smoke test) on `website/**`; `push` to `main`
(production `wrangler deploy`) on `website/**`. This matches the existing Rust
`ci.yml` trigger shape (`pull_request:[main]`, `push:[main]`) — the website
workflow differs only by the `website/**` path filter that keeps the two
pipelines disjoint.

## Wave: DEVOPS / [REF] Coexistence Matrix

| Existing pipeline | File | Website impact | Status |
|---|---|---|---|
| lefthook pre-commit | `lefthook.yml` | none (Rust-only commands; don't match `website/**`) | must_not_break |
| lefthook pre-push | `lefthook.yml` | none (Rust-only) | must_not_break |
| Rust CI | `.github/workflows/ci.yml` | runs today on every push/PR to main (no path filter) — wasteful on website-only changes but NOT broken | must_not_break |
| Rust nightly | `.github/workflows/nightly.yml` | none (scheduled full-workspace mutants) | must_not_break |
| GitHub Pages deploy | `.github/workflows/deploy-pages.yml` | SUPERSEDED by the Workers deploy | supersede in DELIVER slice 01 |

**Separation contract**: the website CI never runs Rust gates (cargo, nextest,
dst, mutants); the Rust CI never runs website gates (tsc, eslint, next build,
wrangler). No shared job, runner state, or cache key. The new website workflow
is path-scoped to `website/**`; the Rust workflows are unchanged.

**Optional, non-breaking improvement (surfaced for the user — not applied):**
`ci.yml` today runs the full ~15-min Rust suite on website-only PRs/pushes. Adding
`paths-ignore: ['website/**', 'docs/**']` (or a `paths:` allowlist) to `ci.yml`
would skip the Rust gates on website-only changes. This edits the Rust workflow,
so it is left as a recommendation for the user to decide — it is NOT required
for correctness (a green-but-wasteful Rust run does not break anything).

**`deploy-pages.yml` supersession finding**: the workflow publishes repo root
(`.`) to GitHub Pages on push to `main` — a generic whole-repo static publish
(it serves the repo-root `index.html`, not a built docs site). It IS active
today. The docs platform deploys to Cloudflare Workers instead, so GitHub Pages
is superseded. **Removal is scheduled for DELIVER slice 01 (skeleton-deploy),
alongside the working Cloudflare deploy** — never remove the only deploy before
its replacement exists. NOT removed in this design wave.

## Wave: DEVOPS / [REF] Pre-requisites (DESIGN constraints + concrete infra tasks)

DESIGN constraints the platform must satisfy (the deploy config must encode all
of these):

- **Node runtime everywhere** — never `export const runtime = 'edge'` (C-2);
  OpenNext manages the Workers Node runtime.
- **`nodejs_compat`** — compatibility flag set; compat date ≥ 2025-09-01.
- **SSG / no runtime `fs`** (C-3) — MDX compiled into the bundle at build time;
  no R2 ISR cache binding.
- **Bundle ceiling** — compressed Worker bundle ≤ 3 MiB Free / 10 MiB Paid (C-8).
- **Isolate ceiling** — in-Worker Orama index shares the 128 MB isolate
  (C-8, ADR-0057); benchmark the external-search migration trigger (>~5k pages
  or ~60–70 MB) against the real corpus before treating it as committed.
- **D1 binding** — `tool_calls` table binding declared in the wrangler config
  (ADR-0056).
- **`SITE_ORIGIN`** — single config constant; `workers.dev` → `overdrive.sh` is
  one flip (D-F).

Concrete remaining DEVOPS / DELIVER infra tasks (stated as scope, not "wire
later" hand-waves; no GitHub issues invented):

1. **Custom-domain DNS + Workers custom-domain binding** for `overdrive.sh`,
   plus the `SITE_ORIGIN` flip (D-F). DEVOPS-infra; sequenced with/after DELIVER
   slice 01.
2. **D1 schema migration** — create the `tool_calls` table (schema in
   `kpi-contracts.yaml`). DELIVER slice 06 (the analytics slice) or its DEVOPS
   prerequisite.
3. **Secrets** — `CLOUDFLARE_API_TOKEN` (scoped to Workers deploy) and
   `CLOUDFLARE_ACCOUNT_ID`, referenced as GitHub Actions repository or
   environment (`production`/`preview`) secrets by the deploy workflow. Create
   the scoped token in Cloudflare and add it as a GH secret — DEVOPS-infra task;
   no values invented.

## Wave: DEVOPS / [REF] Changed Assumptions

None. Every DEVOPS decision was locked before the wave and aligns with the
DESIGN SSOT (D-A..D-H, ADRs 0055–0058) and the brief's docs-platform section.
No DESIGN assumption, story, or AC was altered; no
`devops/upstream-changes.md` was required.

## Wave: DELIVER / [REF] Implementation Summary

The `docs-platform` website shipped end-to-end across 8 thin slices into the
greenfield TypeScript/Next.js subtree at **`website/`** (56 tracked files),
EXEMPT from the Rust gates per C-5. Stack as designed: Fumadocs v16.9 + Next
16.2 (App Router/RSC) + React 19, deployed via `@opennextjs/cloudflare` 1.19 to
Cloudflare Workers (Node runtime, never edge). Delivered LEAN (no DES log /
roadmap.json / mutation gate — DISTILL was intentionally skipped; the four glue
checks were folded into the slices that need them). All slices: typecheck +
eslint + `next build` + `opennextjs-cloudflare build` green, committed
individually. **The real Cloudflare deploy is pending the user's account/token**
(slice 01 landed build + local-workerd serve + the deploy workflow; no live URL
yet).

## Wave: DELIVER / [REF] Slices Shipped

| Slice | Commit | Shipped |
|---|---|---|
| 01 Skeleton | `8f644c2e` | Fumadocs+Next+OpenNext scaffold, one `/docs` page, shared `baseOptions()` shell, `lib/source.ts`; OpenNext build + local workerd serve verified |
| 02 Docs + nav | `fc82b1f6` | Real content (`content/docs/`: intent/observation + DST concepts + `cargo dst` how-to), `meta.json` tree, sidebar/TOC, cross-links (C-6: only real behaviour, verified vs the real `dst` binary) |
| 03 Search | `279abb4c` | In-Worker Orama via `lib/search.ts` seam (exposes `server.search`), `app/api/search/route.ts`, Cmd+K `RootProvider` dialog |
| 04 LLM export + assertion | `b8b9cb25` | `lib/get-llm-text.ts` seam, `/llms.txt`, `/llms-full.txt`, per-page `.md` (`app/api/md/[[...slug]]`), and `scripts/assert-one-index.ts` (ADR-0058, falsifiable — proven by injected violation) |
| 05 MCP server | `252f7ab5` | `app/mcp/route.ts` stateless Streamable HTTP (`WebStandardStreamableHTTPServerTransport`), `search_docs` + `get_doc` reusing the seams; real MCP-client acceptance test |
| 06 Analytics (D1) | `8dd065d2` | D1 `tool_calls` best-effort log via `getCloudflareContext()` + `ctx.waitUntil`; `migrations/0001_tool_calls.sql`; maintainer zero-result query; genuine C-7 fault-injection test |
| 07 Blog | `312ea948` | Second collection joining the ONE index (combined Orama index over docs+published blog); hand-rolled list/post; single `publishedBlogPages()` draft gate; assertion extended |
| 08 Landing | `c13756f3` | `app/(home)/page.tsx` HomeLayout landing under the shared shell, content ported from `index.html`; removed the slice-01 redirect; cold-load value prop in server HTML |

(Plus `c1e2dd56` — scope the package name to `@overdrive/website`.)

## Wave: DELIVER / [REF] Glue-Check Results (the four kept checks)

The acceptance approach agreed in place of a full DISTILL suite: test the
invisible glue, not library behaviour. All four pass.

1. **One-index assertion (C-4/KPI-3, ADR-0058)** — `bun run assert:one-index`,
   wired into the build. 7 published pages (2 blog) reachable from all 3
   consumers (.md, llms.txt, search index); 1 draft excluded from all 3.
   Falsifiable: an injected violation fails the build.
2. **MCP behaviour + contract (`test:mcp`)** — lists exactly `search_docs` +
   `get_doc`; search top-hit correct; **`get_doc(url)` byte-identical to the
   `.md` export** (US-05 identity, 4007==4007 bytes); honest not-found.
3. **C-7 best-effort logging (`test:mcp:analytics`)** — a tool call writes a D1
   row; a zero-result query logs `result_count=0`; **a genuinely induced D1
   failure leaves the tool response byte-identical and undelayed** (21ms/5ms),
   with zero rows in the broken table.
4. **Blog join + draft exclusion (`test:blog`)** — a published-post term is
   found via both `/api/search` and MCP; a draft-only term returns nothing
   anywhere; `/blog` omits the draft.

Deploy smoke is local-workerd (`wrangler dev` → `GET /docs`,`/`,`/blog` → 200);
the live-URL smoke is pending Cloudflare auth.

## Wave: DELIVER / [REF] Deploy Status & Remaining Infra Tasks

Pending the user's Cloudflare account (not blockers to the code, which is
deploy-ready):
- **Cloudflare auth** — `wrangler` login / `CLOUDFLARE_API_TOKEN` + account id
  as GH Actions secrets; then `bun run deploy`.
- **Custom domain** — `overdrive.sh` DNS + Workers binding; flip `SITE_ORIGIN`
  (`lib/site.ts`) from `workers.dev`.
- **D1 production binding** — `wrangler.jsonc` `database_id` is a local-dev
  placeholder; create the real D1 db + apply `migrations/0001_tool_calls.sql`.
- **`deploy-pages.yml` removal** — DEVOPS scheduled its removal alongside the
  working Cloudflare deploy; deferred to when auth lands (never remove the only
  deploy before its replacement is live). STILL PRESENT.
- **Benchmark ADR-0057's in-Worker Orama threshold** against the real corpus.
- Deferrals untouched (no issues created): RSS/OG (D-4); `fumadocs-openapi`
  out of scope (D-5); KPI-2/6 approximated from CF Web Analytics page-view
  funnels (D-D).

## Wave: DELIVER / [REF] Pre-requisites

Consumed: the DISTILL-substitute glue checks (defined in DESIGN/this wave), the
DESIGN component decomposition + ADRs 0055–0058, and `environments.yaml`. No
roadmap.json / execution-log.json (lean non-DES delivery for the exempt
subtree).

---

## Changelog

| Date | Change |
|---|---|
| 2026-05-30 | Initial DISCUSS wave for docs-platform: JTBD (J-DOCS-001/002/003), scope split into 8 slices (walking skeleton = slice 01), two-arc journey, 8 LeanUX stories with embedded AC + KPIs, DoR validation, wave decisions, deferrals surfaced. |
| 2026-05-30 | DESIGN wave (GUIDE mode, pass 2): appended DESIGN sections (Wave Decisions, DDD verdicts D-A..D-H, component decomposition, driving/driven ports, technology choices, Reuse Analysis, open questions, Changed Assumptions=none). Authoritative prose + C4 (L1/L2/L3) in `docs/product/architecture/brief.md` § docs-platform website. ADRs 0055 (MCP same-Worker), 0056 (D1 + best-effort logging), 0057 (in-Worker Orama + seam + benchmarked trigger), 0058 (one-index assertion). D-2 resolved (D1). D-1 recast as benchmarked threshold. D-5 (`fumadocs-openapi`) confirmed OUT OF SCOPE (non-goal). — Morgan. |
| 2026-05-30 | DEVOPS wave (lean / Tier-1): appended DEVOPS sections (Wave Decisions V-1..V-9, Environment Matrix, CI/CD Pipeline Outline, Monitoring Contracts KPI→instrument, Deployment Strategy, Mutation Testing Strategy=disabled-for-website, Observability Stack, Branching Strategy, Coexistence Matrix, Pre-requisites, Changed Assumptions=none). Machine artifact `environments.yaml` (local-dev/preview/production + coexistence matrix). SSOT `docs/product/kpi-contracts.yaml` (per-KPI data collection, D1 `tool_calls` schema, Web Analytics funnels, alert thresholds). Locked decisions: Cloudflare Workers via OpenNext; GitHub Actions CI path-scoped to `website/**` (coexists with Rust `ci.yml`/`nightly.yml`, supersedes `deploy-pages.yml`); Cloudflare-native observability (Workers Logs + Web Analytics + D1); atomic deploy + instant `wrangler` rollback; no deploy-level experimentation; GitHub Flow; mutation testing DISABLED for `website/`. `deploy-pages.yml` (active whole-repo GitHub Pages publish) superseded — removal scheduled for DELIVER slice 01, NOT this wave. Concrete remaining infra tasks stated (custom-domain wiring + `SITE_ORIGIN` flip; D1 `tool_calls` schema migration; `CLOUDFLARE_API_TOKEN`/account-id GH secrets). CLAUDE.md Rust mutation section UNTOUCHED. Forge per-wave review skipped (no novel deploy target). — Apex. |
| 2026-05-30 | DELIVER wave (LEAN, non-DES — DISTILL skipped by agreement; the four glue checks folded into slices). Shipped the `website/` Next+OpenNext subtree across 8 committed slices (`8f644c2e`..`c13756f3`): skeleton, docs+nav, Orama search, llms export + one-index assertion, MCP server, D1 analytics (C-7), blog (one-index join + draft gate), landing. All slices green on typecheck+eslint+next build+opennext build. Four glue checks pass (one-index falsifiable; MCP `get_doc`===`.md`; genuine C-7 fault-injection; blog join + draft exclusion). Real Cloudflare deploy pending user auth (slice 01 = build + local-workerd serve + deploy workflow). Remaining infra tasks + untouched deferrals enumerated in the DELIVER § above. Implemented by nw-software-crafter; orchestrated lean (no roadmap/execution-log/mutation). |
