import Link from "next/link";
import { HoverImpeller } from "@/components/brand";

export function MarketingFooter() {
	return <footer className="od-footer" data-brand-footer>
		<div className="od-footer-top">
			<HoverImpeller className="od-footer-impeller" />
			<div className="od-footer-links">
				<Link href="/docs">Docs</Link>
				<Link href="/docs/concepts/architecture">Architecture</Link>
				<Link href="/docs/comparisons">Comparisons</Link>
				<Link href="/blog">Blog</Link>
				<a href="https://github.com/overdrive-sh/overdrive">GitHub ↗</a>
			</div>
		</div>
		<div className="od-footer-word" aria-hidden>overdrive</div>
		<div className="od-footer-bottom"><span>Overdrive</span><span>Source-available · pre-production</span><a href="#top" aria-label="Back to top">↑</a></div>
	</footer>;
}
