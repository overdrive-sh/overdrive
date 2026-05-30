import { createMDX } from "fumadocs-mdx/next";
import type { NextConfig } from "next";

const nextConfig: NextConfig = {
	// Pin the workspace root to website/ — the repo root carries its own
	// bun.lock for Rust-adjacent tooling, which would otherwise make Next infer
	// the wrong root.
	turbopack: {
		root: __dirname,
	},
	// Docs site: skip Next image optimization (DESIGN D-G). OpenNext manages the
	// runtime; never set `runtime = 'edge'` (research § Decision, C-2).
	images: {
		unoptimized: true,
	},
	// Per-page `.md` export routing (slice 04 / US-04). Next 16 cannot express a
	// literal `.md` suffix on a catch-all segment as a per-page dynamic route
	// (a `[[...slug]].md` / `[...slug].md` folder collapses to a static literal
	// that never matches per-page `.md` URLs, which then 404 through the greedy
	// `/docs/[[...slug]]` page route under OpenNext/workerd). The robust shape is
	// a rewrite at the routing layer OpenNext honors, pointing every `.md` URL at
	// the clean `app/api/md/[[...slug]]` catch-all that runs the `getLLMText`
	// seam. Order matters: the bare `/docs.md` (index) rule precedes the nested
	// `/docs/:path*.md` rule.
	async rewrites() {
		return [
			{ source: "/docs.md", destination: "/api/md" },
			{ source: "/docs/:path*.md", destination: "/api/md/:path*" },
		];
	},
};

const withMDX = createMDX();

export default withMDX(nextConfig);

// Enable calling `getCloudflareContext()` in `next dev`.
// See https://opennext.js.org/cloudflare/bindings#local-access-to-bindings.
import { initOpenNextCloudflareForDev } from "@opennextjs/cloudflare";
initOpenNextCloudflareForDev();
