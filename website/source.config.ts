import { defineDocs } from 'fumadocs-mdx/config';

// The ONE content source (DISCUSS C-4). Slice 01 declares only the docs
// collection; the blog collection is added in slice 07.
export const docs = defineDocs({
  dir: 'content/docs',
});
