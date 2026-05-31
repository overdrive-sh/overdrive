"use client";

import { useEffect, useId, useState } from "react";

// Client-side Mermaid renderer for MDX docs. Mermaid is imported
// dynamically inside an effect so it never runs during SSR (and never
// ships in the server bundle) — required for the OpenNext / Cloudflare
// Workers target. Theme tracks Fumadocs' `.dark` class on <html>, which
// next-themes toggles, via a MutationObserver so the diagram re-renders
// on theme switch without a reload.
export function Mermaid({ chart }: { chart: string }) {
	const id = useId();
	const [svg, setSvg] = useState("");

	useEffect(() => {
		let cancelled = false;

		async function render() {
			const { default: mermaid } = await import("mermaid");
			const isDark = document.documentElement.classList.contains("dark");
			mermaid.initialize({
				startOnLoad: false,
				securityLevel: "loose",
				theme: isDark ? "dark" : "default",
				fontFamily: "inherit",
				// Drop Mermaid's default grey fills so the diagram reads on the
				// brutalist dark canvas: edge-label "pills" and subgraph/cluster
				// boxes become transparent (containers keep only their outline).
				themeVariables: {
					edgeLabelBackground: "transparent",
					clusterBkg: "transparent",
				},
			});
			try {
				const rendered = await mermaid.render(
					`mmd-${id.replace(/[^a-zA-Z0-9]/g, "")}`,
					chart,
				);
				if (!cancelled) setSvg(rendered.svg);
			} catch {
				// Leave the previous render in place on a transient parse error.
			}
		}

		void render();

		const observer = new MutationObserver(() => void render());
		observer.observe(document.documentElement, {
			attributes: true,
			attributeFilter: ["class"],
		});

		return () => {
			cancelled = true;
			observer.disconnect();
		};
	}, [chart, id]);

	return (
		<div
			className="my-6 flex justify-center [&_svg]:h-auto [&_svg]:max-w-full"
			// biome-ignore lint/security/noDangerouslySetInnerHtml: mermaid output is generated from a trusted, in-repo chart string
			dangerouslySetInnerHTML={{ __html: svg }}
		/>
	);
}
