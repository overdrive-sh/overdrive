import "./globals.css";
import { RootProvider } from "fumadocs-ui/provider/next";
import {
	IBM_Plex_Sans,
	IBM_Plex_Mono,
	Instrument_Serif,
} from "next/font/google";
import type { ReactNode } from "react";

/*
  Brand fonts, ported from index.html (IBM Plex Sans / Mono + Instrument Serif).
  `next/font` self-hosts them at build time (no runtime Google Fonts request,
  no layout shift) and exposes CSS variables that `app/globals.css` maps onto
  Tailwind's `--font-sans` / `--font-mono` / `--font-serif` theme keys.
*/
const plexSans = IBM_Plex_Sans({
	subsets: ["latin"],
	weight: ["400", "500", "600", "700"],
	variable: "--font-plex-sans",
	display: "swap",
});

const plexMono = IBM_Plex_Mono({
	subsets: ["latin"],
	weight: ["400", "500", "600"],
	variable: "--font-plex-mono",
	display: "swap",
});

const instrumentSerif = Instrument_Serif({
	subsets: ["latin"],
	weight: "400",
	style: ["normal", "italic"],
	variable: "--font-instrument-serif",
	display: "swap",
});

export default function RootLayout({ children }: { children: ReactNode }) {
	return (
		<html
			lang="en"
			// The brand is dark-only (index.html has no light mode); start in
			// `dark` so the first server-rendered paint is already on-brand.
			className={`dark ${plexSans.variable} ${plexMono.variable} ${instrumentSerif.variable}`}
			suppressHydrationWarning
		>
			<body className="flex flex-col min-h-screen font-sans">
				{/*
					Wire the default Fumadocs fetch-based SearchDialog (Cmd+K) at the
					`/api/search` route handler, which is served by the ONE-index seam in
					`lib/search.ts`. `enabled` + `api` are Fumadocs defaults (true /
					"/api/search"); set explicitly so the search wiring is self-documenting
					and verifiable in the build rather than implicit.

					`theme` forces dark and disables the system preference — the brutalist
					Overdrive brand is dark-only, matching index.html.
				*/}
				<RootProvider
					theme={{
						defaultTheme: "dark",
						forcedTheme: "dark",
						enableSystem: false,
					}}
					search={{
						enabled: true,
						options: { type: "fetch", api: "/api/search" },
					}}
				>
					{children}
				</RootProvider>
			</body>
		</html>
	);
}
