# Slice 01 — Skeleton Deploy (Walking Skeleton)

**This is the walking skeleton.** Thinnest possible end-to-end slice: a
minimal Fumadocs + Next.js (App Router/RSC) site, ONE docs page, the shared
`baseOptions()` nav shell, deployed to Cloudflare Workers via OpenNext —
**live at a real URL**.

## Learning hypothesis

> We believe the chosen stack (Fumadocs on Next.js App Router, deployed to
> Cloudflare Workers via `@opennextjs/cloudflare`) deploys and serves a real
> page at a real URL. We will know this is true when an anonymous visitor
> opens the deployed URL and sees a rendered docs page — proving the
> build → deploy → serve pipeline works before we invest in any feature.

This de-risks the single most fatal assumption: that OpenNext-on-Workers
actually serves Fumadocs at all. Everything downstream builds on it.

## User-visible value (slice-composition gate)

The dogfood moment: **the page is reachable at a real, public URL.** A human
can open it and see Overdrive docs rendered. That is the value — not infra
for its own sake. Maps to J-DOCS-001 (the first authoritative page exists).

## Story

- **US-01** — Skeleton docs site live at a real URL (J-DOCS-001)

## IN scope

- `npm create cloudflare@latest -- <name> --framework=next --platform=workers`
  scaffold (wires OpenNext, Wrangler, `nodejs_compat`).
- ONE docs page at `/docs` (placeholder content, e.g. "Overdrive docs").
- The shared `baseOptions()` shell (the nav shell all three surfaces reuse).
- `createMDX()` from `fumadocs-mdx/next` wrapping `next.config`; docs catch-all
  at `app/docs/[[...slug]]/page.tsx`; `lib/source.ts` over the Next-emitted source.
- **Subtree location decision** (DECIDED 2026-05-30): `website/` at repo root, OUTSIDE
  `crates/`. Exempt from Rust crate-class/dst-lint/nextest/cargo-mutants gates.
- A deploy smoke test (the slice's own quality gate): `wrangler deploy` then
  an HTTP GET on the deployed URL returns 200 with the page content.
- Node runtime everywhere — never `runtime = 'edge'`.

## OUT of scope (explicitly)

- Real docs content / navigation tree (slice 02).
- Search (slice 03), llms export (04), MCP (05/06), blog (07), landing (08).
- ISR/revalidate cache binding (docs are SSG; not needed).
- `next/image` optimization (use `unoptimized` for now).
- Custom domain wiring beyond what the scaffold/Wrangler provides (workers.dev
  URL is acceptable for the skeleton; custom domain is a follow-up — see
  deferral note in feature-delta.md).

## Acceptance (verifies the elevator pitch After→sees)

- `GET https://<deployed-url>/docs` returns 200 and the rendered page body
  contains the placeholder docs heading.
- The nav shell from `baseOptions()` renders on the page.
- The deploy smoke test passes in the slice's own pipeline.

## Effort

≤ 1 day. Single deliverable, demonstrable in one session.

## Dependencies

None (this is the skeleton). All later slices depend on this.
