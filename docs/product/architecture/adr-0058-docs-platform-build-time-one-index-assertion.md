# ADR-0058 — docs-platform: build-time one-index enforcement assertion (the C-4 invariant made structural)

## Status

Accepted. 2026-05-30. Decision-makers: Morgan (proposing); user (GUIDE-mode
decision D-H). Tags: docs-platform, website, overdrive.sh, integration-invariant,
enforcement, earned-trust, application-arch.

**Scope note**: overdrive.sh website (`website/` subtree); architecturally
independent of the Rust platform. This is a **Node/TypeScript build step in
`website/`**, NOT a Rust gate — it never touches `crates/`, dst-lint, or the
xtask harness.

References DISCUSS C-4 (one index, multiple consumers), US-04/US-05/US-07 ACs,
the journey's `docs_content_index` and `page_llm_text` shared artifacts, and the
methodology's enforceable-architecture-rules principle (CLAUDE.md / nWave
principle 11) and Earned Trust (principle 12).

## Context

C-4 — "search, MCP, and llms export all consume the SAME build-time `source`
index; no parallel indexes" — is the strategic invariant of the entire feature.
ADR-0055 (same-Worker MCP) and ADR-0057 (`lib/search.ts` seam) make divergence
*structurally unlikely* by funnelling all consumers through one in-process index
and one query seam. But "unlikely by construction" is a convention; conventions
erode (nWave principle 11). The Earned Trust principle (12) is explicit: a
dependency you assert but do not probe is an act of faith. The one-index claim
is exactly such a dependency — and it spans several surfaces (search index,
`llms.txt`, per-page `.md`, MCP, blog) that a future content or refactor change
could silently break.

The failure modes the invariant guards against are concrete:

- A `source.getPages()` page that has no reachable `.md` export (US-04 AC:
  "100% of pages have a reachable `.md`").
- A page missing from `llms.txt` (US-04 AC).
- A page present in the corpus but absent from the search index (C-4 — the
  human/agent search would miss it).
- A blog post (US-07) that renders but did NOT join the one index — so it is
  invisible to search (US-03) and MCP `search_docs` (US-05). US-07 AC requires a
  unique term in a post to be found by both surfaces.
- `get_doc(url)` output drifting from the `.md` export for the same page (US-05
  AC: byte-identical).

## Decision

Add a **build-time assertion** to the `website/` build — a Node script / build
step (e.g. `website/scripts/assert-one-index.ts`, wired into the `build`
pipeline so a failure fails the build) — that enforces, over the single
`source.getPages()` enumeration:

1. **Every** `source.getPages()` page (docs AND blog) has a **reachable `.md`**
   export (the `getLLMText` path resolves and is non-empty).
2. **Every** page **appears in `llms.txt`** (the llms index covers the full
   corpus — no page is silently omitted).
3. **Every** page **is present in the search index** that `lib/search.ts`
   queries (the Orama index built from the same `source`).
4. **Blog posts are in the same index** as docs (US-07 / C-4 — the blog
   collection joined the one index, not a parallel one).

The assertion enumerates the corpus ONCE from `source.getPages()` and checks
each consumer against that single enumeration — so it cannot itself introduce a
second source of truth. A failure is a hard build failure with a structured
message naming the offending page and the consumer it is missing from.

This is the Earned-Trust probe for the C-4 invariant: it empirically
demonstrates, on every build, that the "one index, N consumers" claim holds —
rather than trusting that the seam and the topology kept it true.

**Optionally (encouraged, low-cost)**: assert that `get_doc(url)` output equals
the `.md` export for a sample (or all) pages, closing the US-05 byte-identity AC
structurally. Both call `getLLMText`, so this is a cheap equality check over the
shared primitive.

## Alternatives considered

### Alternative A — Rely on the seam + topology alone, no assertion (REJECTED — the pass-1 implicit default)

Trust that the same-Worker single-index topology (ADR-0055) and the
`lib/search.ts` seam (ADR-0057) keep all consumers aligned.

- **Why considered**: the topology already makes divergence structurally
  unlikely; no extra code.
- **Why rejected**: "structurally unlikely" is not "verified." A content edit,
  a blog-collection misconfiguration, a `meta.json` mistake, or a future
  refactor could omit a page from one consumer while the build stays green. C-4
  is the feature's most important invariant; per nWave principle 11
  (enforceable rules) and principle 12 (Earned Trust), an invariant without an
  automated probe erodes. The assertion is the structural enforcement that
  upgrades the convention to a guarantee.

### Alternative B — A Rust dst-lint-style gate (REJECTED — wrong toolchain)

- **Why rejected**: the website is a TypeScript subtree exempt from the Rust
  gates (DISCUSS C-5). A Rust gate cannot see `source.getPages()` (a build-time
  JS/TS artifact). The enforcement must live where the index lives — a Node
  build step in `website/`. (Decision D-H is explicit on this.)

### Alternative C — Runtime check on each request (REJECTED)

- **Why rejected**: the divergence is a build-time / content-shape property; a
  runtime check would burn isolate cycles per request to detect a condition that
  is fixed at build time. Build-time is the correct altitude — fail the deploy,
  never serve a divergent index.

## Consequences

**Positive**

- C-4 becomes a structural guarantee, verified on every build — the single most
  important invariant of the feature can no longer silently regress.
- Catches the concrete US-04/US-05/US-07 AC failure modes (orphaned `.md`,
  missing `llms.txt` entry, un-indexed page, blog post not in the one index)
  before deploy.
- It is the website's analogue of the Rust dst-lint gate: enforceable
  architecture rule in the toolchain the artifact actually uses.

**Negative / accepted trade-offs**

- One more build step to maintain in `website/`. Low cost; it is pure
  enumeration + set membership over the already-built `source`.
- The assertion is itself a dependency that could rot (e.g. stop checking a new
  consumer added later). Per Earned-Trust self-application (principle 12): when
  a new consumer of the one index is added, the assertion gains a corresponding
  check in the same change — the build step is the place the "did we add a
  consumer without wiring it to the index?" question is answered.
