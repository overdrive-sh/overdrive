import Link from "next/link";
import {
	Eyebrow,
	Glyph,
	PlannedPill,
	PrimaryCta,
	SecondaryCta,
} from "@/components/marketing";

// The marketing landing page at `/`, rendered inside `app/(home)/layout.tsx`
// (`HomeLayout` + `baseOptions()`) so `/`, `/docs`, and `/blog` share ONE nav
// shell (the baseOptions_shell invariant).
//
// Structure is inspired by modern developer-platform landing pages (animated
// hero, an "included not assembled" strip, a bento capability grid, product-UI
// mock panels, a comparison, a stat band, a closing CTA). The COPY and ASSETS
// are original Overdrive — brand colours, brand voice, real behaviour only.
//
// Voice: sell OUTCOMES, not internals. No component names (etcd, CNI, Envoy,
// SPIRE), no implementation primitives (reconcilers, Raft, XDP, kTLS, SPIFFE
// in prose), no unmeasured performance numbers. Every line says what the
// reader GETS. Claims are grounded in real or explicitly-designed behaviour
// from docs/product and the whitepaper (C-6: real content only, no invented
// features, nothing presented as measured that hasn't been). Anything not yet
// shipped lives in a visually distinct, explicitly-labelled "on the roadmap"
// block.
//
// Statically rendered (no "use client") so the hero value prop is in the
// server-rendered HTML on first load; all motion is CSS-only (see globals.css
// `od-*` keyframes) and disabled under prefers-reduced-motion.

export const metadata = {
	title: "Overdrive — Everything you run, on one platform",
	description:
		"Deploy web services, background jobs, virtual machines, and functions on one platform — encrypted, load-balanced, and self-healing from the first deploy, on your own hardware.",
};

// The workload kinds Overdrive runs under one control plane. Used as hero
// chips and in the "Run anything" tile.
const workloadKinds = [
	"Services",
	"Jobs",
	"microVMs",
	"VMs",
	"WASM",
] as const;

// The marquee band under the hero — what ships in the box. Honest, named
// capabilities, no fake customer logos.
const includedStrip = [
	"Service discovery",
	"Load balancing",
	"Mutual TLS",
	"Workload identity",
	"Network policy",
	"Health checks",
	"Flow telemetry",
	"Credential proxy",
	"Self-healing",
	"One-file deploy",
	"Sealed appliance OS",
] as const;

// Factual, non-performance numbers — safe under C-6 (no benchmark claims).
const stats = [
	{ value: "5", label: "workload kinds, one control plane" },
	{ value: "1", label: "file to describe an app" },
	{ value: "0", label: "sidecars to inject" },
	{ value: "mTLS", label: "on by default, no certs to rotate by hand" },
] as const;

// What you get — outcomes, not internals. `span` drives the bento layout so a
// few tiles read larger; the order tiles cleanly into 4 full rows on `lg`.
const capabilities: {
	tag: string;
	title: string;
	body: string;
	span?: boolean;
}[] = [
	{
		tag: "RUN",
		title: "Run anything",
		body: "Long-running services, batch jobs, microVMs, full VMs, and WebAssembly functions run side by side under one control plane — not a separate stack for the workloads that don't fit in a container.",
		span: true,
	},
	{
		tag: "SECURITY",
		title: "Mutual TLS by default",
		body: "Every service-to-service connection is mTLS-encrypted automatically — on from the first deploy, with no sidecar to inject and no certificates to rotate by hand.",
	},
	{
		tag: "IDENTITY",
		title: "Authorized by identity, not IP",
		body: "Each workload gets a short-lived cryptographic identity from a built-in CA. You write policy against what a service is, not the address it happens to hold today.",
	},
	{
		tag: "NETWORKING",
		title: "Networking, built in",
		body: "Service discovery, load balancing across healthy backends, and network policy ship with the platform. No separate ingress controller or external load balancer to stand up and keep in sync.",
	},
	{
		tag: "RELIABILITY",
		title: "Health-checked and restarted",
		body: "Readiness and liveness probes gate traffic and catch failures; an allocation that fails its liveness check restarts, and the platform holds the replica count you declared.",
	},
	{
		tag: "OBSERVABILITY",
		title: "Flow telemetry, no agent",
		body: "Per-connection and per-request telemetry, tagged with the workload identity on each side, with no sidecar to run and no code to instrument. You see what actually talked to what.",
		span: true,
	},
	{
		tag: "SECRETS",
		title: "Workloads never hold the keys",
		body: "A credential proxy holds the real secrets and signs outbound requests on a workload's behalf; the process only ever sees a handle. An AI agent can't exfiltrate a key it was never given.",
	},
	{
		tag: "DEPLOY",
		title: "Ship from one file",
		body: "Describe an app in a single TOML spec and deploy it with one command. Deploy is idempotent on the spec's content hash — an identical spec is a no-op, safe to run straight from CI.",
	},
	{
		tag: "INFRASTRUCTURE",
		title: "Runs as a sealed appliance",
		body: "Nodes boot an immutable, minimal OS image — no shell, no package manager, no SSH. Your hardware, locked down like an appliance, instead of a general-purpose distro you harden and hope stays hardened.",
		span: true,
	},
] as const;

// The "included, not assembled" contrast — the usual à-la-carte stack vs. one
// platform. Each row is a capability you otherwise wire together yourself.
const assembledVsIncluded = [
	"Service mesh for encryption and identity",
	"Ingress controller and external load balancer",
	"Certificate issuance and rotation",
	"A secrets manager and injection sidecar",
	"A separate runtime for VMs and functions",
	"Per-connection network telemetry",
] as const;

// The intended push-to-release path, rendered as the Deployments pipeline
// ribbon. Planned, not shipped (C-6).
const deployPipeline = [
	"Push",
	"Build",
	"Release",
	"Preview",
	"Rollback",
] as const;

// Intended, not shipped. Today's real primitive is `overdrive deploy <spec>`;
// this is where deploy is going — building and releasing from source, with
// previews and rollbacks. Rendered in the marked "on the roadmap" block (C-6).
const roadmap = [
	{
		title: "Push to deploy",
		body: "Push your source and Overdrive builds the image and releases it — no separate CI to wire up before the first deploy.",
	},
	{
		title: "Build pipeline",
		body: "Turn a repository into a runnable, reproducible artifact on the platform, without hand-managing a registry.",
	},
	{
		title: "Preview environments",
		body: "Every branch or pull request gets its own isolated environment, created on push and torn down on merge.",
	},
	{
		title: "Instant rollback",
		body: "Releases are discrete, promotable versions, so reverting a bad deploy is one command rather than a fresh redeploy.",
	},
] as const;

// Self-healing, the whitepaper §12 way: tiered remediation ending in an LLM SRE
// agent. Intended, not shipped — rendered in the marked "on the roadmap" block.
const selfHealingTiers = [
	{
		title: "Reflexive",
		body: "Dead backends are routed around in-kernel and resource pressure is relieved before an OOM kill — in milliseconds, with no control-plane round trip.",
	},
	{
		title: "Reactive",
		body: "A crashed allocation reschedules onto healthy capacity and an unhealthy node drains its workloads, converging back to the state you declared.",
	},
	{
		title: "Reasoning",
		body: "An SRE agent correlates signals by workload identity, finds the root cause, and proposes typed remediations through a risk-based approval gate — learning from every past incident.",
	},
] as const;

const faqs = [
	{
		q: "Is it production-ready?",
		a: "No. Overdrive runs on a single node today and is pre-production. The docs mark every behaviour still at the design stage rather than describing it as shipped.",
	},
	{
		q: "Is it open source?",
		a: "It is source-available under FSL-1.1, and each release converts to the Apache 2.0 licence two years after it ships.",
	},
	{
		q: "Do I need a service mesh, an ingress controller, or cert-manager?",
		a: "No. Encryption, identity, service discovery, load balancing, and network policy are part of the platform and on by default — there is nothing extra to install or keep in sync.",
	},
	{
		q: "What can I run on it?",
		a: "Long-running services, batch jobs, microVMs, full VMs, and WebAssembly functions — five workload kinds under one control plane, described in one spec format.",
	},
	{
		q: "How does it compare to Kubernetes, Nomad, or Fly.io?",
		a: "The comparison page makes the case and the counter-case plainly, including where Overdrive is the wrong choice today.",
	},
] as const;

export default function HomePage() {
	return (
		<main className="flex flex-1 flex-col">
			{/* ───────────────────────── Hero ───────────────────────── */}
			<section className="relative overflow-hidden border-b border-fd-border">
				{/* Warm radial glow, slow-pulsing, at the top of the hero. */}
				<div
					aria-hidden
					className="od-glow pointer-events-none absolute left-1/2 top-[-200px] z-0 h-[760px] w-[1000px] max-w-none -translate-x-1/2"
					style={{
						background:
							"radial-gradient(ellipse, rgba(255,92,40,0.12) 0%, transparent 62%)",
					}}
				/>
				{/* Floating pixel cubes, hidden on narrow viewports. */}
				<svg
					aria-hidden
					className="od-float pointer-events-none absolute left-[4%] top-[20%] z-0 hidden opacity-55 xl:block"
					width="120"
					height="160"
					viewBox="0 0 120 160"
					fill="none"
				>
					<rect x="0" y="0" width="22" height="22" fill="#ff5c28" opacity="0.7" />
					<rect x="24" y="0" width="22" height="22" fill="#ff8b5f" opacity="0.35" />
					<rect x="0" y="24" width="22" height="22" fill="#3a3a44" />
					<rect x="24" y="48" width="22" height="22" fill="#ff5c28" opacity="0.45" />
					<rect x="48" y="72" width="22" height="22" fill="#2a2a31" />
					<rect x="24" y="96" width="22" height="22" fill="#ff5c28" opacity="0.6" />
					<rect x="0" y="120" width="22" height="22" fill="#3a3a44" />
				</svg>
				<svg
					aria-hidden
					className="od-float-alt pointer-events-none absolute right-[4%] top-[34%] z-0 hidden opacity-55 xl:block"
					width="140"
					height="160"
					viewBox="0 0 140 160"
					fill="none"
				>
					<rect x="118" y="0" width="22" height="22" fill="#ff5c28" opacity="0.5" />
					<rect x="94" y="24" width="22" height="22" fill="#2a2a31" />
					<rect x="118" y="48" width="22" height="22" fill="#ff8b5f" opacity="0.4" />
					<rect x="70" y="72" width="22" height="22" fill="#ff5c28" opacity="0.7" />
					<rect x="94" y="96" width="22" height="22" fill="#3a3a44" />
					<rect x="118" y="120" width="22" height="22" fill="#ff5c28" opacity="0.45" />
				</svg>

				<div className="container relative z-10 mx-auto max-w-5xl px-4 py-20 text-center md:py-32">
					{/* Status badge */}
					<div className="mb-6 inline-flex items-center gap-2 border border-fd-border bg-fd-card/60 px-3 py-1.5 font-mono text-xs text-fd-muted-foreground">
						<span className="od-dot inline-block h-1.5 w-1.5 bg-[var(--color-brand)]" />
						Source-available · single-node · pre-production
					</div>

					<h1 className="mx-auto max-w-4xl font-serif text-5xl font-normal leading-[1.04] tracking-tight md:text-7xl">
						Everything you run, on one platform.
					</h1>
					<p className="mx-auto mt-6 max-w-2xl text-lg text-fd-muted-foreground md:text-xl">
						Deploy long-running services, batch jobs, microVMs, and
						WebAssembly functions — with mutual TLS, load balancing, and
						self-healing built in. One platform to operate on your own
						hardware, instead of a stack you assemble and babysit.
					</p>

					<div className="mt-8 flex flex-wrap justify-center gap-4">
						<PrimaryCta href="/docs/how-to/deploy-a-workload">
							Deploy a workload
						</PrimaryCta>
						<SecondaryCta href="/docs">Read the docs</SecondaryCta>
					</div>

					{/* Workload-kind chips */}
					<div className="mt-10 flex flex-wrap justify-center gap-2">
						{workloadKinds.map((k) => (
							<span
								key={k}
								className="border border-fd-border bg-fd-card/50 px-3 py-1 font-mono text-xs text-fd-muted-foreground"
							>
								{k}
							</span>
						))}
					</div>
				</div>

				{/* Included strip — a seamless marquee of what ships in the box. */}
				<div className="relative z-10 overflow-hidden border-t border-fd-border bg-fd-card/40 py-4">
					<div className="flex w-max od-marquee gap-3 pr-3">
						{[...includedStrip, ...includedStrip].map((item, i) => (
							<span
								key={`${item}-${i}`}
								className="flex items-center gap-2 whitespace-nowrap font-mono text-xs uppercase tracking-widest text-fd-muted-foreground"
							>
								<span className="text-[var(--color-brand)]">·</span>
								{item}
							</span>
						))}
					</div>
				</div>
			</section>

			{/* ───────────────────── Stat band ───────────────────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto grid max-w-6xl grid-cols-2 gap-px overflow-hidden border-x border-fd-border bg-fd-border md:grid-cols-4">
					{stats.map((s) => (
						<div key={s.label} className="bg-fd-background p-6 text-center md:p-8">
							<div className="font-serif text-4xl text-fd-foreground md:text-5xl">
								{s.value}
							</div>
							<div className="mt-2 text-xs text-fd-muted-foreground">
								{s.label}
							</div>
						</div>
					))}
				</div>
			</section>

			{/* ─────────────── Included, not assembled ─────────────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto grid max-w-6xl items-center gap-10 px-4 py-16 md:grid-cols-2 md:py-24">
					<div>
						<Eyebrow>{"// One platform"}</Eyebrow>
						<h2 className="mb-4 font-serif text-3xl font-normal md:text-4xl">
							Included, not assembled.
						</h2>
						<p className="mb-6 max-w-xl text-fd-muted-foreground">
							The hard parts of running an app — encryption, identity, load
							balancing, secrets, telemetry — are usually six products you
							pick, wire together, and keep in sync. Overdrive ships them as
							one platform, on by default. There is nothing to bolt on before
							the first deploy.
						</p>
						<Link
							href="/docs/comparisons"
							className="inline-block font-medium text-[var(--color-brand)] hover:underline"
						>
							See how it compares →
						</Link>
					</div>

					{/* The à-la-carte stack collapses into one box. */}
					<div className="grid gap-4">
						{assembledVsIncluded.map((item) => (
							<div
								key={item}
								className="flex items-center justify-between gap-4 border border-fd-border bg-fd-card px-4 py-3"
							>
								<span className="text-sm text-fd-muted-foreground line-through decoration-fd-border">
									{item}
								</span>
								<span className="flex shrink-0 items-center gap-2 font-mono text-xs uppercase tracking-widest text-[var(--color-brand)]">
									<Glyph size={14} />
									Included
								</span>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* ─────────────── Capability bento grid ─────────────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-6xl px-4 py-16 md:py-24">
					<Eyebrow>{"// What you get"}</Eyebrow>
					<h2 className="mb-3 max-w-3xl font-serif text-3xl font-normal md:text-4xl">
						Everything you need to run an app, already in the box.
					</h2>
					<p className="mb-10 max-w-2xl text-fd-muted-foreground">
						Networking, security, identity, and reliability aren&apos;t
						add-ons you pick, wire together, and keep running. They&apos;re
						part of the platform — on by default, from the first deploy.
					</p>
					<div className="grid grid-flow-row-dense gap-px overflow-hidden border border-fd-border bg-fd-border sm:grid-cols-2 lg:grid-cols-3">
						{capabilities.map((c) => (
							<div
								key={c.title}
								className={`od-card group bg-fd-background p-6 transition-colors hover:bg-fd-muted md:p-8 ${
									c.span ? "sm:col-span-2 lg:col-span-2" : ""
								}`}
							>
								<div className="mb-4 flex items-center gap-3">
									<Glyph />
									<span className="font-mono text-[10px] uppercase tracking-widest text-fd-muted-foreground">
										{c.tag}
									</span>
								</div>
								<h3 className="mb-2 text-lg font-semibold">{c.title}</h3>
								<p className="max-w-xl text-sm text-fd-muted-foreground">
									{c.body}
								</p>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* ─────────────── Deploy flow (product UX) ─────────────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto grid max-w-6xl items-center gap-10 px-4 py-16 md:grid-cols-2 md:py-24">
					<div>
						<Eyebrow>{"// Ship it"}</Eyebrow>
						<h2 className="mb-3 font-serif text-3xl font-normal md:text-4xl">
							From a file to a running app.
						</h2>
						<p className="mb-6 text-fd-muted-foreground">
							Describe a workload in one TOML file — what to run, the CPU and
							memory it gets, and the health checks that tell the platform
							when it&apos;s ready. Deploy it with one command and watch it
							converge. No YAML templating, no apply-then-wait dance.
						</p>
						<ul className="space-y-3 text-sm text-fd-muted-foreground">
							<li className="flex gap-3">
								<span className="font-mono text-[var(--color-brand)]">1</span>
								<span>
									<code className="font-mono text-fd-foreground">
										overdrive deploy
									</code>{" "}
									ships the spec and streams progress until your app is
									running and healthy.
								</span>
							</li>
							<li className="flex gap-3">
								<span className="font-mono text-[var(--color-brand)]">2</span>
								<span>
									Re-run it any time — deploy is idempotent, so an identical
									spec changes nothing and a CI job that can&apos;t tell
									whether it already landed is safe to run anyway.
								</span>
							</li>
						</ul>
						<Link
							href="/docs/how-to/deploy-a-workload"
							className="mt-6 inline-block font-medium text-[var(--color-brand)] hover:underline"
						>
							Read the deploy guide →
						</Link>
					</div>

					{/* Terminal mock: payments.toml → deploy → running. */}
					<div className="overflow-hidden border border-fd-border bg-fd-card font-mono text-sm shadow-[0_0_60px_rgba(255,92,40,0.06)]">
						<div className="flex items-center gap-2 border-b border-fd-border px-4 py-2 text-xs text-fd-muted-foreground">
							<span className="h-2.5 w-2.5 bg-[var(--color-brand)] opacity-80" />
							<span className="h-2.5 w-2.5 bg-fd-border" />
							<span className="h-2.5 w-2.5 bg-fd-border" />
							<span className="ml-2">payments.toml</span>
						</div>
						<pre className="overflow-x-auto p-4 leading-relaxed">
							<code>{`[service]
id       = "payments"
replicas = 1

[exec]
command = "/opt/payments/bin/server"

[[listener]]
port = 8080

[[health_check.readiness]]
type = "http"
path = "/healthz"
port = 8080`}</code>
						</pre>
						<div className="border-t border-fd-border px-4 py-3 leading-relaxed text-fd-muted-foreground">
							<div>
								<span className="text-[var(--color-brand)]">$</span> overdrive
								deploy payments.toml
							</div>
							<div className="mt-1">payments · deploying…</div>
							<div>
								payments · <span className="text-fd-foreground">running</span>
								&nbsp;&nbsp;1/1 healthy
							</div>
						</div>
					</div>
				</div>
			</section>

			{/* ─────────── Deep dive: zero-trust networking ─────────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto grid max-w-6xl items-center gap-10 px-4 py-16 md:grid-cols-2 md:py-24">
					{/* Topology mock: identity-to-identity, mTLS, policy verdict. */}
					<div className="order-2 overflow-hidden border border-fd-border bg-fd-card md:order-1">
						<div className="border-b border-fd-border px-4 py-2 font-mono text-xs text-fd-muted-foreground">
							service topology
						</div>
						<div className="space-y-3 p-5 font-mono text-xs">
							{[
								{ from: "web", to: "payments", verdict: "allow" },
								{ from: "web", to: "sessions", verdict: "allow" },
								{ from: "payments", to: "ledger", verdict: "allow" },
								{ from: "guest", to: "ledger", verdict: "deny" },
							].map((edge) => (
								<div
									key={`${edge.from}-${edge.to}`}
									className="flex items-center justify-between border border-fd-border bg-fd-background px-3 py-2.5"
								>
									<span className="flex items-center gap-2">
										<span className="text-fd-foreground">{edge.from}</span>
										<span className="text-fd-muted-foreground">→</span>
										<span className="text-fd-foreground">{edge.to}</span>
									</span>
									<span className="flex items-center gap-3">
										<span className="text-fd-muted-foreground">mTLS</span>
										<span
											className={
												edge.verdict === "allow"
													? "text-[var(--color-brand)]"
													: "text-fd-muted-foreground line-through"
											}
										>
											{edge.verdict}
										</span>
									</span>
								</div>
							))}
							<p className="pt-1 text-[10px] uppercase tracking-widest text-fd-muted-foreground">
								Policy is written against identity, enforced per connection.
							</p>
						</div>
					</div>

					<div className="order-1 md:order-2">
						<Eyebrow>{"// Zero trust, by default"}</Eyebrow>
						<h2 className="mb-3 font-serif text-3xl font-normal md:text-4xl">
							Encrypted and authorized, with nothing to wire up.
						</h2>
						<p className="mb-6 text-fd-muted-foreground">
							Every connection between your services is mutually authenticated
							and encrypted from the first deploy. Each workload carries a
							short-lived cryptographic identity, so you write policy against
							what a service <em>is</em> — not the IP address it happens to
							hold today. No sidecar to inject, no mesh to operate, no
							certificates to rotate by hand.
						</p>
						<ul className="space-y-2 text-sm text-fd-muted-foreground">
							<li className="flex items-start gap-3">
								<span className="mt-1">
									<Glyph size={14} />
								</span>
								mTLS on every service-to-service hop, automatically.
							</li>
							<li className="flex items-start gap-3">
								<span className="mt-1">
									<Glyph size={14} />
								</span>
								Identities issued and rotated by a built-in CA.
							</li>
							<li className="flex items-start gap-3">
								<span className="mt-1">
									<Glyph size={14} />
								</span>
								Network policy that survives a workload moving hosts.
							</li>
						</ul>
					</div>
				</div>
			</section>

			{/* ─────── Deep dive: secrets proxy + flow telemetry ─────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto grid max-w-6xl items-center gap-10 px-4 py-16 md:grid-cols-2 md:py-24">
					<div>
						<Eyebrow>{"// The workload never holds the keys"}</Eyebrow>
						<h2 className="mb-3 font-serif text-3xl font-normal md:text-4xl">
							Secrets it can&apos;t leak, telemetry you didn&apos;t instrument.
						</h2>
						<p className="mb-6 text-fd-muted-foreground">
							A credential proxy holds the real secrets and signs outbound
							requests on a workload&apos;s behalf — the process only ever
							sees a handle, so an AI agent can&apos;t exfiltrate a key it was
							never given. And because every connection already flows through
							the platform, you get per-request telemetry tagged with the
							identity on each side, with no agent to run and no code to
							instrument.
						</p>
						<Link
							href="/docs"
							className="inline-block font-medium text-[var(--color-brand)] hover:underline"
						>
							Explore the platform →
						</Link>
					</div>

					{/* Flow telemetry mock: identity-tagged request rows. */}
					<div className="overflow-hidden border border-fd-border bg-fd-card">
						<div className="flex items-center justify-between border-b border-fd-border px-4 py-2 font-mono text-xs text-fd-muted-foreground">
							<span>flow telemetry</span>
							<span className="flex items-center gap-1.5">
								<span className="od-dot inline-block h-1.5 w-1.5 bg-[var(--color-brand)]" />
								live
							</span>
						</div>
						<div className="divide-y divide-fd-border font-mono text-xs">
							{[
								{ src: "web", dst: "payments", code: "200", ms: "12ms" },
								{ src: "payments", dst: "ledger", code: "200", ms: "31ms" },
								{ src: "web", dst: "sessions", code: "200", ms: "4ms" },
								{ src: "payments", dst: "vault", code: "200", ms: "8ms" },
								{ src: "web", dst: "payments", code: "503", ms: "—" },
							].map((row, i) => (
								<div
									key={i}
									className="flex items-center justify-between px-4 py-2.5"
								>
									<span className="flex items-center gap-2">
										<span className="text-fd-foreground">{row.src}</span>
										<span className="text-fd-muted-foreground">→</span>
										<span className="text-fd-foreground">{row.dst}</span>
									</span>
									<span className="flex items-center gap-4 text-fd-muted-foreground">
										<span
											className={
												row.code === "200"
													? "text-[var(--color-brand)]"
													: "text-fd-foreground"
											}
										>
											{row.code}
										</span>
										<span className="w-12 text-right">{row.ms}</span>
									</span>
								</div>
							))}
						</div>
					</div>
				</div>
			</section>

			{/* ─────── On the roadmap: Deployments (own section) ─────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-6xl px-4 py-16 md:py-24">
					<Eyebrow accent={false}>{"// On the roadmap · Deployments"}</Eyebrow>
					<h2 className="mb-3 max-w-3xl font-serif text-3xl font-normal md:text-4xl">
						From your source, not just a spec.
					</h2>
					<p className="mb-8 max-w-2xl text-fd-muted-foreground">
						Push your source and Overdrive will build it, release it, preview
						every branch, and roll back in one command.
					</p>

					{/* Pipeline ribbon — the intended push-to-release path. */}
					<div className="mb-12 flex flex-wrap items-center gap-x-2 gap-y-3">
						{deployPipeline.map((step, i) => (
							<span key={step} className="flex items-center gap-2">
								<span className="flex items-center gap-2 border border-dashed border-fd-border bg-fd-card px-3 py-1.5 font-mono text-xs uppercase tracking-widest text-fd-muted-foreground">
									<span className="text-[var(--color-brand)]">{`0${i + 1}`}</span>
									{step}
								</span>
								{i < deployPipeline.length - 1 && (
									<span aria-hidden className="text-fd-muted-foreground">
										→
									</span>
								)}
							</span>
						))}
					</div>

					{/* Copy + mock "releases" panel (illustrative, clearly planned). */}
					<div className="grid items-center gap-10 md:grid-cols-2">
						<div>
							<ul className="space-y-3 text-sm text-fd-muted-foreground">
								<li className="flex items-start gap-3">
									<span className="mt-1">
										<Glyph size={14} />
									</span>
									Push a repository and Overdrive builds a reproducible
									artifact — no separate CI to stand up first.
								</li>
								<li className="flex items-start gap-3">
									<span className="mt-1">
										<Glyph size={14} />
									</span>
									Each build becomes a discrete, promotable release, and every
									branch gets its own isolated preview environment.
								</li>
								<li className="flex items-start gap-3">
									<span className="mt-1">
										<Glyph size={14} />
									</span>
									Reverting a bad deploy is one command against a prior
									release — not a fresh redeploy.
								</li>
							</ul>
							<Link
								href="/docs/how-to/deploy-a-workload"
								className="mt-6 inline-block font-medium text-[var(--color-brand)] hover:underline"
							>
								Deploy from a spec today →
							</Link>
						</div>

						<div className="overflow-hidden border border-dashed border-fd-border bg-fd-card font-mono text-sm">
							<div className="flex items-center justify-between border-b border-dashed border-fd-border px-4 py-2 text-xs text-fd-muted-foreground">
								<span>releases</span>
								<span className="uppercase tracking-widest">
									Illustrative · planned
								</span>
							</div>
							<div className="px-4 py-3 leading-relaxed text-fd-muted-foreground">
								<div>
									<span className="text-[var(--color-brand)]">$</span> git push
									overdrive main
								</div>
								<div className="mt-1">building payments…</div>
								<div>
									released <span className="text-fd-foreground">v3</span>
									&nbsp;&nbsp;1/1 healthy
								</div>
							</div>
							<div className="divide-y divide-dashed divide-fd-border border-t border-dashed border-fd-border text-xs">
								{[
									{ v: "v3", when: "just now", state: "current" },
									{ v: "v2", when: "3 days ago", state: "rollback" },
									{ v: "v1", when: "last week", state: "rollback" },
								].map((r) => (
									<div
										key={r.v}
										className="flex items-center justify-between px-4 py-2.5"
									>
										<span className="flex items-center gap-3">
											<span className="text-fd-foreground">{r.v}</span>
											<span className="text-fd-muted-foreground">{r.when}</span>
										</span>
										{r.state === "current" ? (
											<span className="text-[var(--color-brand)] uppercase tracking-widest">
												serving
											</span>
										) : (
											<span className="border border-fd-border px-2 py-0.5 uppercase tracking-widest text-fd-muted-foreground">
												rollback
											</span>
										)}
									</div>
								))}
							</div>
						</div>
					</div>

					{/* What lands with it — the four roadmap capabilities. */}
					<div className="mt-12 grid gap-px overflow-hidden border border-dashed border-fd-border bg-fd-border sm:grid-cols-2 lg:grid-cols-4">
						{roadmap.map((r) => (
							<div key={r.title} className="bg-fd-background p-6">
								<PlannedPill />
								<h3 className="mb-2 text-base font-semibold">{r.title}</h3>
								<p className="text-sm text-fd-muted-foreground">{r.body}</p>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* ─────── On the roadmap: Self-healing (own section) ─────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-6xl px-4 py-16 md:py-24">
					<Eyebrow accent={false}>{"// On the roadmap · Self-healing"}</Eyebrow>
					<h2 className="mb-3 max-w-3xl font-serif text-3xl font-normal md:text-4xl">
						Tiered, ending in an SRE agent.
					</h2>
					<p className="mb-8 max-w-2xl text-fd-muted-foreground">
						Restarting a failed instance is the easy tier. The intended end
						state investigates: correlate across the fleet by workload
						identity, find the root cause, and apply typed fixes through a
						risk-based approval gate — auto-applying the safe ones, asking
						before the risky ones, and remembering every resolved incident.
					</p>

					{/* Escalation ribbon — each tier catches what the one below can't. */}
					<div className="mb-12 flex flex-wrap items-center gap-x-2 gap-y-3">
						{selfHealingTiers.map((t, i) => (
							<span key={t.title} className="flex items-center gap-2">
								<span className="flex items-center gap-2 border border-dashed border-fd-border bg-fd-card px-3 py-1.5 font-mono text-xs uppercase tracking-widest text-fd-muted-foreground">
									<span className="text-[var(--color-brand)]">{`0${i + 1}`}</span>
									{t.title}
								</span>
								{i < selfHealingTiers.length - 1 && (
									<span aria-hidden className="text-fd-muted-foreground">
										→
									</span>
								)}
							</span>
						))}
					</div>

					{/* Copy + mock "incident" panel (illustrative, clearly planned). */}
					<div className="grid items-center gap-10 md:grid-cols-2">
						<div>
							<ul className="space-y-3 text-sm text-fd-muted-foreground">
								<li className="flex items-start gap-3">
									<span className="mt-1">
										<Glyph size={14} />
									</span>
									Each tier catches what the one below it can&apos;t —
									escalation, not duplication.
								</li>
								<li className="flex items-start gap-3">
									<span className="mt-1">
										<Glyph size={14} />
									</span>
									The fast tiers act in the data path and the scheduler, with
									no human in the loop and no control-plane round trip.
								</li>
								<li className="flex items-start gap-3">
									<span className="mt-1">
										<Glyph size={14} />
									</span>
									The reasoning tier proposes rather than acts unprompted: safe
									fixes auto-apply, risky ones wait for you.
								</li>
							</ul>
							<Link
								href="/docs/how-to/deploy-a-workload"
								className="mt-6 inline-block font-medium text-[var(--color-brand)] hover:underline"
							>
								Health checks ship today →
							</Link>
						</div>

						<div className="overflow-hidden border border-dashed border-fd-border bg-fd-card font-mono text-sm">
							<div className="flex items-center justify-between border-b border-dashed border-fd-border px-4 py-2 text-xs text-fd-muted-foreground">
								<span>incident</span>
								<span className="uppercase tracking-widest">
									Illustrative · planned
								</span>
							</div>
							<div className="px-4 py-3 leading-relaxed text-fd-muted-foreground">
								<div>payments · liveness failing 1/3</div>
								<div className="mt-1">
									<span className="text-[var(--color-brand)]">reflexive</span>{" "}
									routed around the dead backend
								</div>
								<div>
									<span className="text-[var(--color-brand)]">reactive</span>{" "}
									rescheduled onto healthy capacity
								</div>
							</div>
							<div className="divide-y divide-dashed divide-fd-border border-t border-dashed border-fd-border text-xs">
								{[
									{
										stage: "reasoning",
										what: "correlated by identity",
										state: "root cause found",
										kind: "found",
									},
									{
										stage: "proposed",
										what: "roll back payments → v2",
										state: "awaiting approval",
										kind: "pending",
									},
								].map((r) => (
									<div
										key={r.stage}
										className="flex items-center justify-between gap-3 px-4 py-2.5"
									>
										<span className="flex items-center gap-3">
											<span className="text-fd-foreground">{r.stage}</span>
											<span className="text-fd-muted-foreground">{r.what}</span>
										</span>
										{r.kind === "found" ? (
											<span className="shrink-0 text-[var(--color-brand)] uppercase tracking-widest">
												{r.state}
											</span>
										) : (
											<span className="shrink-0 border border-fd-border px-2 py-0.5 uppercase tracking-widest text-fd-muted-foreground">
												{r.state}
											</span>
										)}
									</div>
								))}
							</div>
						</div>
					</div>

					{/* What runs at each tier. */}
					<div className="mt-12 grid gap-px overflow-hidden border border-dashed border-fd-border bg-fd-border sm:grid-cols-2 lg:grid-cols-3">
						{selfHealingTiers.map((t) => (
							<div key={t.title} className="bg-fd-background p-6">
								<PlannedPill />
								<h3 className="mb-2 text-base font-semibold">{t.title}</h3>
								<p className="text-sm text-fd-muted-foreground">{t.body}</p>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* ───────────────────────── FAQ ───────────────────────── */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-6xl px-4 py-16 md:py-24">
					<Eyebrow>{"// Straight answers"}</Eyebrow>
					<h2 className="mb-10 max-w-3xl font-serif text-3xl font-normal md:text-4xl">
						Questions worth asking up front.
					</h2>
					<div className="grid gap-px overflow-hidden border border-fd-border bg-fd-border md:grid-cols-2">
						{faqs.map((f) => (
							<div key={f.q} className="bg-fd-background p-6 md:p-8">
								<h3 className="mb-2 flex items-start gap-3 text-base font-semibold">
									<span className="mt-1">
										<Glyph size={14} />
									</span>
									{f.q}
								</h3>
								<p className="text-sm text-fd-muted-foreground">{f.a}</p>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* ─────────────── Honest status + CTA ─────────────── */}
			<section className="relative overflow-hidden">
				<div
					aria-hidden
					className="od-glow pointer-events-none absolute left-1/2 top-1/2 z-0 h-[600px] w-[900px] max-w-none -translate-x-1/2 -translate-y-1/2"
					style={{
						background:
							"radial-gradient(ellipse, rgba(255,92,40,0.10) 0%, transparent 62%)",
					}}
				/>
				<div className="container relative z-10 mx-auto max-w-4xl px-4 py-24 text-center">
					<Eyebrow>{"// Where it is today"}</Eyebrow>
					<h2 className="mb-4 font-serif text-4xl font-normal md:text-5xl">
						Early, single-node, and honest about it.
					</h2>
					<p className="mx-auto mb-8 max-w-2xl text-fd-muted-foreground">
						Overdrive runs on a single node today and is pre-production. The
						docs mark every behaviour still at the design stage rather than
						describing it as shipped, and there are no benchmark numbers on
						this page because there is nothing at fleet scale to measure yet.
						It is source-available, and converts to a permissive open-source
						licence two years after each release. If you are weighing it
						against Kubernetes, Nomad, or Fly.io, the comparison page makes the
						case and the counter-case plainly.
					</p>
					<div className="flex flex-wrap justify-center gap-4">
						<PrimaryCta href="/docs/how-to/deploy-a-workload">
							Deploy a workload
						</PrimaryCta>
						<SecondaryCta href="/docs/comparisons">
							See how it compares
						</SecondaryCta>
					</div>
				</div>
			</section>
		</main>
	);
}
