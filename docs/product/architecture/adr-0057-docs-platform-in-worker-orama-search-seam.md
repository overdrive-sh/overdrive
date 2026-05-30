# ADR-0057 — docs-platform: in-Worker Orama search behind a `lib/search.ts` seam, with a benchmarked external-search migration trigger

## Status

Accepted. 2026-05-30. Decision-makers: Morgan (proposing); user (GUIDE-mode
decision D-C). Tags: docs-platform, website, overdrive.sh, search, seam,
integration-invariant, application-arch.

**Scope note**: overdrive.sh website (`website/` subtree); architecturally
independent of the Rust platform. Resolves part of DISCUSS Deferral D-1
(external search migration) by recording the trigger as a benchmarked design
threshold rather than a "wire later" promise.

References DISCUSS US-03 (J-DOCS-001), constraints C-4 (one index), C-8 (bundle
+ 128 MB isolate ceiling), and research Findings 2.1.1, 2.3.1, 2.3.2, 3.4.4.

## Context

US-03 (J-DOCS-001) requires sub-second docs search: Priya presses Cmd+K, types
a technical term, and the top hit is the relevant page. The same index must
serve the MCP `search_docs` tool (US-05) and underlie the llms export (US-04) —
the C-4 one-index invariant.

Research records three search architectures, in ascending edge-friendliness
(Findings 2.1.1, 2.3.x): in-Worker Orama (`createFromSource`, the index lives
in the isolate's memory) → static export (browser downloads + builds the index)
→ external Orama Cloud / Algolia (build-time `sync()`; zero isolate index). The
in-Worker path shares the hard 128 MB isolate ceiling (Finding 3.4.4) with the
app, the bundled content, and the MCP handler.

The critical numbers from Finding 3.4.4 (and their confidence):

- **128 MB per-isolate memory limit is the load-bearing hard wall** —
  authoritative Cloudflare limit, High confidence.
- Orama's in-memory footprint measured ≈0.5 KB/doc (≈5 MB per 10k docs) on one
  **synthetic** 2023 corpus — Medium-High confidence for bounding reasoning,
  and explicitly **NOT an authoritative "max docs in a Worker" number**. Real
  prose docs index larger than random tokens.
- The doc-count thresholds derived from these (low tens of thousands of pages
  as the prudent in-Worker ceiling) are **inference that MUST be benchmarked
  against the real overdrive.sh corpus** before they are treated as committed.

The Overdrive docs corpus at launch is a focused set (hundreds of pages at
most) — comfortably single-digit MB of index, well inside in-Worker viability.

## Decision

**Use in-Worker Orama now** via Fumadocs' documented Next default —
`app/api/search/route.ts` with `export const { GET } = createFromSource(source)`
(Node runtime, C-2) — over the ONE build-time `source` index (C-4).

**Behind a thin `lib/search.ts` seam.** Both `/api/search` (the browser dialog)
and the MCP `search_docs` tool (ADR-0055) call ONE function
`searchIndex(query)` exported from `lib/search.ts`. Neither consumer talks to
Orama directly. The seam:

- **Reinforces C-4**: there is exactly one query path; the human dialog and the
  agent tool cannot diverge because they call the identical function.
- **Makes the future external swap a single-file change**: migrating to Orama
  Cloud / Algolia (or static export) replaces the body of `searchIndex(query)`
  in `lib/search.ts` and swaps the client `SearchDialog` config (research
  Finding 2.4.2's `search.SearchDialog` escape hatch) — no change at either call
  site.

**Migration trigger (benchmarked threshold, NOT a published number).** Move
from in-Worker Orama to an external build-time-synced index (Orama Cloud or
Algolia, research Findings 2.3.1/2.3.2) when EITHER holds:

- the corpus exceeds **~5,000 pages** (inference, to be benchmarked against the
  real corpus — labelled inference per Finding 3.4.4, not a committed limit); OR
- the in-Worker index footprint approaches **~60–70 MB** of the 128 MB isolate
  (i.e. leaving prudent headroom for the runtime + app + bundled content + the
  MCP handler that share the isolate).

Both figures are **inference pending a benchmark against the actual
overdrive.sh corpus** — they are design thresholds to watch, not load-bearing
numbers. The 128 MB isolate limit is the only hard, authoritative figure here.
When the trigger fires, the change is localised to `lib/search.ts` + the client
search-dialog config; this is a property of the seam, not a scheduled follow-up.

## Alternatives considered

### Alternative A — External search (Orama Cloud / Algolia) from day one (REJECTED — the pass-1 alternative)

Build-time `sync()` to an external index, queried by the client; zero isolate
index.

- **Why considered**: the most edge-friendly architecture; the eventual
  destination if the corpus grows; sidesteps the 128 MB ceiling entirely.
- **Why rejected for now**: it adds an external service dependency, API-key
  management, and a build-time sync step for a corpus that is comfortably
  single-digit MB in-Worker today. That is complexity (and an external
  integration to contract-test) paid up-front against a ceiling the launch
  corpus is nowhere near. The `lib/search.ts` seam means deferring it costs
  nothing structurally — the swap is a single-file change when the benchmarked
  trigger fires. Adopting it now would be optimising for a scale the feature
  does not have, against the "simplest solution first" default.

### Alternative B — Static export (browser builds the index) (REJECTED for now, retained as fallback)

`staticGET` + client `type: 'static'` (research Finding 2.2.1).

- **Why considered**: zero runtime search compute on the isolate; most
  edge-friendly of the self-hosted options.
- **Why rejected for now**: trades isolate memory for client bandwidth/CPU (the
  client downloads + builds the index). For a focused corpus in-Worker is
  simpler and the isolate is uncontended. Retained as the **fallback** path the
  seam can also target (zero isolate index, no external dependency) if the
  external-search dependency is undesirable when the trigger fires.

### Alternative C — Orama directly at both call sites, no seam (REJECTED)

- **Why rejected**: two call sites (`/api/search`, MCP `search_docs`) each
  constructing/querying Orama is the C-4 divergence risk in miniature, and a
  future external migration would be a two-site change. The seam costs one tiny
  module and buys the no-divergence guarantee plus single-file migration.

## Consequences

**Positive**

- Simplest viable search for the launch corpus; no external dependency, no API
  keys, no build-time sync step.
- `searchIndex(query)` is the single query path — C-4 reinforced for the
  search consumers, byte-for-byte the same ranking for human and agent.
- The external-search escape hatch is a documented, benchmarked, single-file
  change — the migration is de-risked without being prematurely taken.

**Negative / accepted trade-offs**

- The in-Worker index shares the 128 MB isolate budget with the app, bundled
  content, and the MCP handler (C-8). Monitored against the benchmarked
  trigger; the launch corpus is far inside the budget.
- The migration trigger numbers are inference until benchmarked — they could
  shift once measured against real prose docs. This is stated as a threshold to
  watch, carried to DEVOPS/DISTILL as a measurement task, not a fixed limit.

## Handoff annotation (DEVOPS wave)

When the external-search migration trigger fires, Orama Cloud / Algolia become
**external integrations** and warrant **consumer-driven contract tests** (e.g.
Pact-JS, since the website is TypeScript) on the build-time `sync()` boundary to
detect breaking API changes before deploy. Not needed at launch (no external
search); flagged for the wave that takes the migration.
