# Slice 08 — Landing Page

The marketing landing page at `/` using Fumadocs `HomeLayout`, with content
copied from the existing `index.html` (placeholder, not a bespoke redesign).

## Learning hypothesis

> We believe the existing index.html marketing content can be rehomed into a
> Fumadocs `HomeLayout` at `/`, sharing the same `baseOptions()` shell as docs
> and blog. We will know this is true when a visitor lands on `/`, sees the
> value prop, and navigates into /docs and /blog through one consistent shell.

## User-visible value

Priya **lands on a real value-prop page** and flows into docs/blog through one
coherent shell — the front door of the whole site. Maps to J-DOCS-001 (first
impression + path into the docs).

## Story

- **US-08** — Land on the marketing home and navigate into docs/blog (J-DOCS-001)

## IN scope

- `/` using `HomeLayout`.
- Marketing content copied/adapted from the existing repo-root `index.html`
  (placeholder content — fidelity is "carries the message", not pixel-perfect).
- The shared `baseOptions()` nav shell linking `/`, `/docs`, `/blog`.
- Node runtime; SSG.

## OUT of scope

- A bespoke marketing redesign — explicitly a placeholder using existing copy.
- New marketing copywriting — reuse index.html's message.
- A/B testing, marketing analytics, lead capture — out of scope for the feature.
- `next/image` optimization (use `unoptimized` or a simple loader).

## Acceptance (verifies elevator pitch After→sees)

- `GET /` returns the `HomeLayout` home with the value prop from index.html.
- The nav shell links to `/docs` and `/blog`, and both navigate correctly.
- The nav shell on `/` is the SAME `baseOptions()` shell as docs and blog
  (no nav drift across surfaces).

## Effort

≤ 1 day.

## Dependencies

- Slice 01 (shell). Best last so the home links to real docs (02) and blog (07).
