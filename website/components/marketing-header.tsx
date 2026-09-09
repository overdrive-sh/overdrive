"use client";

import Link from "next/link";
import { useEffect, useRef, useState } from "react";
import { useHomeLayout } from "fumadocs-ui/layouts/home";

const links = [
	{ href: "/#platform", label: "Platform" },
	{ href: "/#deploy", label: "Deploy" },
	{ href: "/docs", label: "Docs" },
	{ href: "/blog", label: "Blog" },
	{ href: "https://github.com/overdrive-sh/overdrive", label: "GitHub" },
];

export function MarketingHeader() {
	const { slots } = useHomeLayout();
	const [open, setOpen] = useState(false);
	const toggle = useRef<HTMLButtonElement>(null);

	useEffect(() => {
		if (!open) return;
		const onKey = (event: KeyboardEvent) => {
			if (event.key !== "Escape") return;
			setOpen(false);
			toggle.current?.focus();
		};
		const wide = window.matchMedia("(min-width: 901px)");
		const onWide = () => { if (wide.matches) setOpen(false); };
		document.addEventListener("keydown", onKey);
		wide.addEventListener("change", onWide);
		return () => {
			document.removeEventListener("keydown", onKey);
			wide.removeEventListener("change", onWide);
		};
	}, [open]);

	return (
		<>
			<div className="od-announcement" id="top">
				<span>Source-available · single-node · pre-production</span>
				<a href="https://github.com/overdrive-sh/overdrive">GitHub <span aria-hidden>↗</span></a>
			</div>
			<header className="od-header">
				<nav className="od-navigation" aria-label="Primary">
					<div className="od-nav-left">
						{links.slice(0, 3).map(link => <Link key={link.href} href={link.href}>{link.label}</Link>)}
					</div>
					{slots.navTitle && <slots.navTitle className="od-nav-brand" />}
					<div className="od-nav-right">
						<Link href="/blog">Blog</Link>
						{slots.searchTrigger && <slots.searchTrigger.sm hideIfDisabled className="od-search-trigger" />}
						<Link className="od-button od-nav-cta" href="/docs/how-to/deploy-a-workload">Deploy a workload <span aria-hidden>↗</span></Link>
					</div>
					<button ref={toggle} className="od-menu-toggle" aria-label={open ? "Close menu" : "Open menu"} aria-expanded={open} aria-controls="od-mobile-menu" onClick={() => setOpen(!open)}>
						<span /><span />
					</button>
				</nav>
				{open && <nav id="od-mobile-menu" className="od-mobile-menu" aria-label="Mobile">
					{links.map(link => <Link key={link.href} href={link.href} onClick={() => setOpen(false)}>{link.label}<span aria-hidden>↗</span></Link>)}
					{slots.searchTrigger && <slots.searchTrigger.full hideIfDisabled className="od-mobile-search" />}
					<Link className="od-button" href="/docs/how-to/deploy-a-workload" onClick={() => setOpen(false)}>Deploy a workload <span aria-hidden>↗</span></Link>
				</nav>}
			</header>
		</>
	);
}
