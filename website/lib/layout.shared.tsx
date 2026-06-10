import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";

// The shared nav shell. ALL three surfaces — docs, blog, landing — reuse this
// one `baseOptions()` instance (the baseOptions_shell invariant).
//
// The title is the Overdrive wordmark ported from index.html: "OVERDRIVE" in
// uppercase mono followed by the blinking orange cursor block (`.brand-cursor`,
// styled in app/globals.css).
export function baseOptions(): BaseLayoutProps {
	return {
		nav: {
			title: (
				<span className="font-mono text-base font-semibold uppercase tracking-wide">
					Overdrive
					<span className="brand-cursor" aria-hidden="true" />
				</span>
			),
		},
		// Fumadocs renders a GitHub icon link in the nav from this URL, on every
		// surface that uses `baseOptions()` (landing, docs, blog).
		githubUrl: "https://github.com/overdrive-sh/overdrive",
	};
}
