import { defineDocs } from 'fumadocs-mdx/config';

// The ONE content source (DISCUSS C-4). Slice 01 declares only the docs
// collection; the blog collection is added in slice 07.
//
// `postprocess.includeProcessedMarkdown` (slice 04) makes the compiled MDX
// expose `page.data.getText('processed')` — the clean, chrome-free markdown
// rendering that `lib/get-llm-text.ts` reads. Without it, `getText('processed')`
// throws at build time. This is the build-plugin requirement behind the whole
// LLM-export surface (llms.txt / llms-full.txt / per-page `.md`).
export const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
});
