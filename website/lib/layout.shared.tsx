import type { BaseLayoutProps } from "fumadocs-ui/layouts/shared";

// The shared nav shell. ALL three future surfaces — docs, blog, landing —
// reuse this one `baseOptions()` instance (the baseOptions_shell invariant).
// Establishing it now (slice 01) is in-scope per the slice brief.
export function baseOptions(): BaseLayoutProps {
	return {
		nav: {
			title: "Overdrive docs",
		},
	};
}
