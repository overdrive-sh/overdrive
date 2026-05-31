# OpenNext Starter

This is a [Next.js](https://nextjs.org) project bootstrapped with [`create-next-app`](https://nextjs.org/docs/app/api-reference/cli/create-next-app).

## Getting Started

Read the documentation at https://opennext.js.org/cloudflare.

## Develop

Run the Next.js development server:

```bash
npm run dev
# or similar package manager command
```

Open [http://localhost:3000](http://localhost:3000) with your browser to see the result.

You can start editing the page by modifying `app/page.tsx`. The page auto-updates as you edit the file.

## Preview

Preview the application locally on the Cloudflare runtime:

```bash
npm run preview
# or similar package manager command
```

## Deploy

Deploy the application to Cloudflare:

```bash
npm run deploy
# or similar package manager command
```

## MCP tool-call analytics (D1, best-effort — ADR-0056)

The `/mcp` route logs one `{tool, query, ts, result_count}` row per tool call to
a Cloudflare D1 table `tool_calls` (binding `ANALYTICS_DB`). The write is
fire-and-forget via `ctx.waitUntil()` + catch-swallow and MUST NEVER block,
delay, or alter the tool response (C-7 guardrail).

Apply the schema to the local dev D1 (run once before `wrangler dev` / preview):

```bash
bunx wrangler d1 migrations apply ANALYTICS_DB --local   # or: bun run analytics:migrate:local
```

For the provisioned (remote) D1 — DEVOPS wave, once `database_id` is real:

```bash
bunx wrangler d1 migrations apply ANALYTICS_DB --remote
```

Maintainer read path (US-06 / J-DOCS-003) — top zero-result + top queries:

```bash
bun run analytics:top-zero-results            # local D1
bun run analytics:top-zero-results --remote   # provisioned D1
```

The C-7 guardrail + analytics acceptance test:

```bash
bun run test:mcp:analytics
```

## Learn More

To learn more about Next.js, take a look at the following resources:

- [Next.js Documentation](https://nextjs.org/docs) - learn about Next.js features and API.
- [Learn Next.js](https://nextjs.org/learn) - an interactive Next.js tutorial.

You can check out [the Next.js GitHub repository](https://github.com/vercel/next.js) - your feedback and contributions are welcome!
