import { baseOptions } from "@/lib/layout.shared";
import { HomeLayout } from "fumadocs-ui/layouts/home";
import type { ReactNode } from "react";

// The shared nav shell for non-docs pages. The blog (and, later, the landing
// page in slice 08) lives under the SAME `baseOptions()` instance the
// `DocsLayout` uses (the baseOptions_shell invariant) — so the nav title and
// search wiring are identical across /, /docs, and /blog. `HomeLayout` is the
// documented Fumadocs shell for non-docs content (no sidebar/page-tree).
export default function Layout({ children }: { children: ReactNode }) {
	return <HomeLayout {...baseOptions()}>{children}</HomeLayout>;
}
