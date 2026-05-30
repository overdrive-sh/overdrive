import "./globals.css";
import { RootProvider } from "fumadocs-ui/provider/next";
import type { ReactNode } from "react";

export default function RootLayout({ children }: { children: ReactNode }) {
	return (
		<html lang="en" suppressHydrationWarning>
			<body className="flex flex-col min-h-screen">
				{/*
					Wire the default Fumadocs fetch-based SearchDialog (Cmd+K) at the
					`/api/search` route handler, which is served by the ONE-index seam in
					`lib/search.ts`. `enabled` + `api` are Fumadocs defaults (true /
					"/api/search"); set explicitly so the search wiring is self-documenting
					and verifiable in the build rather than implicit.
				*/}
				<RootProvider
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
