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
};

const withMDX = createMDX();

export default withMDX(nextConfig);

// Enable calling `getCloudflareContext()` in `next dev`.
// See https://opennext.js.org/cloudflare/bindings#local-access-to-bindings.
import { initOpenNextCloudflareForDev } from "@opennextjs/cloudflare";
initOpenNextCloudflareForDev();
