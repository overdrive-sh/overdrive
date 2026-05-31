import { defineCollections, defineDocs, frontmatterSchema } from 'fumadocs-mdx/config';
import { z } from 'zod';

// The ONE content engine (DISCUSS C-4). Two collections — `docs` and `blog` —
// are compiled by the same `fumadocs-mdx` pass into the same build-time bundle.
// They are NOT two indexes: `lib/search.ts` builds a SINGLE Orama index over
// both, `lib/get-llm-text.ts` renders both, and `scripts/assert-one-index.ts`
// asserts every page of both is reachable from every consumer. Slice 07 adds
// `blog` as a second `defineCollections({ type: 'doc' })` collection that joins
// these existing consumers (the C-4 "one index, multiple consumers" invariant).
//
// `postprocess.includeProcessedMarkdown` (slice 04) makes the compiled MDX
// expose `page.data.getText('processed')` — the clean, chrome-free markdown
// rendering that `lib/get-llm-text.ts` reads. Without it, `getText('processed')`
// throws at build time. This is the build-plugin requirement behind the whole
// LLM-export surface (llms.txt / llms-full.txt / per-page `.md`). Both
// collections carry it so blog posts export clean markdown identically to docs.
export const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
});

// The blog collection (slice 07). Flat `doc` posts under `content/blog/` — no
// catch-all, no `meta.json` ordering (the list page sorts by `date`). The
// schema extends the built-in page frontmatter (`title`, `description`, …) with
// the blog-specific fields. `draft` is the published-gate: a post with
// `draft: true` is excluded from the list AND from all three index consumers
// (search / llms / MCP). It defaults to `false` so an authored post is
// published unless explicitly held back (DISCUSS DoR 3rd UAT scenario).
export const blog = defineCollections({
  type: 'doc',
  dir: 'content/blog',
  postprocess: {
    includeProcessedMarkdown: true,
  },
  schema: frontmatterSchema.extend({
    date: z.string().date().or(z.date()),
    author: z.string(),
    summary: z.string().optional(),
    draft: z.boolean().default(false),
  }),
});
