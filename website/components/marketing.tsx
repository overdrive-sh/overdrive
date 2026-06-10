import Link from "next/link";

// Shared marketing primitives for the site's non-docs pages (the landing page
// and the deployments roadmap page). All plain server components — no hooks,
// no "use client" — so every page importing them stays statically prerendered.

// A small brand glyph: four rects echoing the hero pixel cubes.
export function Glyph({ size = 20 }: { size?: number }) {
	return (
		<svg
			aria-hidden
			width={size}
			height={size}
			viewBox="0 0 20 20"
			fill="none"
			className="shrink-0"
		>
			<rect x="0" y="0" width="9" height="9" fill="var(--color-brand)" />
			<rect x="11" y="0" width="9" height="9" fill="var(--color-brand)" opacity="0.35" />
			<rect x="0" y="11" width="9" height="9" fill="#3a3a44" />
			<rect x="11" y="11" width="9" height="9" fill="var(--color-brand)" opacity="0.6" />
		</svg>
	);
}

// A mono section eyebrow: `// label`.
export function Eyebrow({
	children,
	accent = true,
}: {
	children: string;
	accent?: boolean;
}) {
	return (
		<p
			className={`mb-3 font-mono text-xs uppercase tracking-widest ${
				accent ? "text-[var(--color-brand)]" : "text-fd-muted-foreground"
			}`}
		>
			{children}
		</p>
	);
}

// A small "Planned" pill for roadmap blocks (C-6: marks not-yet-shipped work).
export function PlannedPill() {
	return (
		<span className="mb-3 inline-block border border-fd-border px-2 py-0.5 font-mono text-[10px] uppercase tracking-widest text-fd-muted-foreground">
			Planned
		</span>
	);
}

// The brand primary CTA button.
export function PrimaryCta({
	href,
	children,
}: {
	href: string;
	children: string;
}) {
	return (
		<Link
			href={href}
			className="rounded-md bg-fd-primary px-6 py-3 font-semibold text-fd-primary-foreground transition-all hover:bg-[var(--color-brand-2)] hover:shadow-[0_0_32px_rgba(255,92,40,0.35)]"
		>
			{children}
		</Link>
	);
}

// The secondary (outline) CTA button.
export function SecondaryCta({
	href,
	children,
}: {
	href: string;
	children: string;
}) {
	return (
		<Link
			href={href}
			className="rounded-md border border-fd-border px-6 py-3 font-medium transition-colors hover:bg-fd-muted"
		>
			{children}
		</Link>
	);
}
