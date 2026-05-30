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

## Changelog

| Date | Change |
|---|---|
| 2026-05-30 | Initial DISCUSS wave for docs-platform: JTBD (J-DOCS-001/002/003), scope split into 8 slices (walking skeleton = slice 01), two-arc journey, 8 LeanUX stories with embedded AC + KPIs, DoR validation, wave decisions, deferrals surfaced. |
