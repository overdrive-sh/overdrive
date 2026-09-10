import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";
import { ScrollLogo } from "@/components/brand";

// The shared nav shell. ALL three surfaces — docs, blog, landing — reuse this
// one `baseOptions()` instance (the baseOptions_shell invariant).
//
// The title is the shared animated Overdrive lockup. ScrollLogo owns its
// client-side motion while this server-side shell keeps the same brand across
// docs, blog, and the landing surface.
export function baseOptions(): BaseLayoutProps {
	return {
		nav: {
			title: <ScrollLogo />,
		},
		// Fumadocs renders a GitHub icon link in the nav from this URL, on every
		// surface that uses `baseOptions()` (landing, docs, blog).
		githubUrl: "https://github.com/overdrive-sh/overdrive",
		// The brand is dark-only (the root layout forces `dark`), so the
		// light/dark appearance toggle is dead UI — hide it.
		themeSwitch: { enabled: false },
	};
}
