import Link from "next/link";

// The marketing landing page at `/`, rendered inside `app/(home)/layout.tsx`
// (`HomeLayout` + `baseOptions()`) so `/`, `/docs`, and `/blog` share ONE nav
// shell (the baseOptions_shell invariant).
//
// Voice: sell OUTCOMES, not internals. No component names (etcd, CNI, Envoy,
// SPIRE), no implementation primitives (reconcilers, Raft, XDP, kTLS, SPIFFE),
// no unmeasured performance numbers. Every line says what the reader GETS.
// Claims are grounded in real or explicitly-designed behaviour from
// docs/product and the whitepaper (C-6: real content only, no invented
// features, nothing presented as measured that hasn't been).
//
// Statically rendered (no "use client") so the hero value prop is in the
// server-rendered HTML on first load.

export const metadata = {
	title: "Overdrive — Everything you run, on one platform",
	description:
		"Deploy web services, background jobs, virtual machines, and functions on one platform — encrypted, load-balanced, and self-healing from the first deploy, on your own hardware.",
};

// A small brutalist square glyph — brand rects, echoing the hero pixel cubes.
function Glyph() {
	return (
		<svg
			aria-hidden
			width="20"
			height="20"
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

// What you get — outcomes, not internals. Each card is something the reader can
// do or rely on, with no component names and no engineering jargon.
const capabilities = [
	{
		tag: "RUN",
		title: "Run anything",
		body: "Long-running services, batch jobs, microVMs, full VMs, and WebAssembly functions run side by side under one control plane — not a separate stack for the workloads that don't fit in a container.",
	},
	{
		tag: "NETWORKING",
		title: "Networking, built in",
		body: "Service discovery, load balancing across healthy backends, and network policy ship with the platform. There is no separate ingress controller or external load balancer to stand up and keep in sync.",
	},
	{
		tag: "SECURITY",
		title: "Mutual TLS by default",
		body: "Every service-to-service connection is mTLS-encrypted automatically — on from the first deploy, with no sidecar to inject and no certificates to rotate by hand.",
	},
	{
		tag: "IDENTITY",
		title: "Authorized by identity, not IP",
		body: "Each workload gets a short-lived cryptographic identity (SPIFFE) from a built-in CA. You write policy against what a service is, not the IP address it happens to hold today.",
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
	},
	{
		tag: "SECRETS",
		title: "Workloads never hold the keys",
		body: "A credential proxy holds the real secrets and signs outbound requests on a workload's behalf; the process only ever sees a handle. An AI agent can't exfiltrate a key it was never given.",
	},
	{
		tag: "DEPLOY",
		title: "Ship from one file",
		body: "Describe an app in a single TOML spec and deploy it with one command. Deploy is idempotent on the spec's content hash — an identical spec is a no-op, so it is safe to run straight from CI.",
	},
	{
		tag: "INFRASTRUCTURE",
		title: "Runs as a sealed appliance",
		body: "Nodes boot an immutable, minimal OS image — no shell, no package manager, no SSH. Your hardware, locked down like an appliance, instead of a general-purpose distro you harden and hope stays hardened.",
	},
] as const;

// Intended, not shipped. Rendered in a visually distinct, explicitly-labelled
// "on the roadmap" block so it can't be read as a shipped capability (C-6).
// Today's real primitive is `overdrive deploy <spec>`; this is where deploy is
// going — building and releasing from source, with previews and rollbacks.
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

export default function HomePage() {
	return (
		<main className="flex flex-1 flex-col">
			{/* Hero */}
			<section className="relative overflow-hidden border-b border-fd-border">
				{/* Warm radial glow at the top of the hero. */}
				<div
					aria-hidden
					className="pointer-events-none absolute left-1/2 top-[-180px] z-0 h-[700px] w-[900px] max-w-none -translate-x-1/2"
					style={{
						background:
							"radial-gradient(ellipse, rgba(255,92,40,0.10) 0%, transparent 60%)",
					}}
				/>
				{/* Floating pixel cubes, hidden on narrow viewports. */}
				<svg
					aria-hidden
					className="pointer-events-none absolute left-[4%] top-[18%] z-0 hidden opacity-55 xl:block"
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
					className="pointer-events-none absolute right-[4%] top-[32%] z-0 hidden opacity-55 xl:block"
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
				<div className="container relative z-10 mx-auto max-w-5xl px-4 py-20 md:py-28">
					<p className="mb-4 font-mono text-xs uppercase tracking-widest text-fd-muted-foreground">
						Services · Jobs · microVMs · WASM
					</p>
					<h1 className="max-w-3xl font-serif text-5xl font-normal leading-[1.05] tracking-tight md:text-7xl">
						Everything you run, on one platform.
					</h1>
					<p className="mt-6 max-w-2xl text-lg text-fd-muted-foreground md:text-xl">
						Deploy long-running services, batch jobs, microVMs, and
						WebAssembly functions — with mutual TLS, load balancing, and
						self-healing built in. One platform to operate on your own
						hardware, instead of a stack you assemble and babysit.
					</p>
					<div className="mt-8 flex flex-wrap gap-4">
						<Link
							href="/docs/how-to/deploy-a-workload"
							className="rounded-md bg-fd-primary px-6 py-3 font-semibold text-fd-primary-foreground transition-all hover:bg-[var(--color-brand-2)] hover:shadow-[0_0_32px_rgba(255,92,40,0.35)]"
						>
							Deploy a workload
						</Link>
						<Link
							href="/docs"
							className="rounded-md border border-fd-border px-6 py-3 font-medium transition-colors hover:bg-fd-muted"
						>
							Read the docs
						</Link>
					</div>
				</div>
			</section>

			{/* What you get — the product surface grid */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-6xl px-4 py-16 md:py-20">
					<p className="mb-3 font-mono text-xs uppercase tracking-widest text-[var(--color-brand)]">
						{"// What you get"}
					</p>
					<h2 className="mb-3 max-w-3xl font-serif text-3xl font-normal md:text-4xl">
						Everything you need to run an app, already included.
					</h2>
					<p className="mb-10 max-w-2xl text-fd-muted-foreground">
						Networking, security, identity, and self-healing aren&apos;t
						add-ons you pick, wire together, and keep running. They&apos;re
						part of the platform — on by default, from the first deploy.
					</p>
					<div className="grid gap-px overflow-hidden border border-fd-border bg-fd-border sm:grid-cols-2 lg:grid-cols-3">
						{capabilities.map((c) => (
							<div
								key={c.title}
								className="group bg-fd-background p-6 transition-colors hover:bg-fd-muted"
							>
								<div className="mb-4 flex items-center gap-3">
									<Glyph />
									<span className="font-mono text-[10px] uppercase tracking-widest text-fd-muted-foreground">
										{c.tag}
									</span>
								</div>
								<h3 className="mb-2 text-lg font-semibold">{c.title}</h3>
								<p className="text-sm text-fd-muted-foreground">{c.body}</p>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* The deploy flow — the actual product UX */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto grid max-w-6xl items-center gap-10 px-4 py-16 md:grid-cols-2 md:py-20">
					<div>
						<p className="mb-3 font-mono text-xs uppercase tracking-widest text-[var(--color-brand)]">
							{"// Ship it"}
						</p>
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
									<code className="font-mono text-fd-foreground">overdrive deploy</code>{" "}
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
					<div className="overflow-hidden border border-fd-border bg-fd-card font-mono text-sm">
						<div className="border-b border-fd-border px-4 py-2 text-xs text-fd-muted-foreground">
							payments.toml
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
								payments ·{" "}
								<span className="text-fd-foreground">running</span>
								&nbsp;&nbsp;1/1 healthy
							</div>
						</div>
					</div>
				</div>
			</section>

			{/* On the roadmap — intended capabilities, clearly marked not-yet-shipped */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-6xl px-4 py-16 md:py-20">
					<p className="mb-3 font-mono text-xs uppercase tracking-widest text-fd-muted-foreground">
						{"// On the roadmap"}
					</p>
					<h2 className="mb-3 max-w-3xl font-serif text-3xl font-normal md:text-4xl">
						What we&apos;re building next.
					</h2>
					<p className="mb-12 max-w-2xl text-fd-muted-foreground">
						Two capabilities we intend to ship, set out here as intent — not
						as shipped behaviour. Everything in this section is marked
						accordingly.
					</p>

					{/* Deployments */}
					<h3 className="mb-1 text-lg font-semibold">
						Deployments — from your source, not just a spec
					</h3>
					<p className="mb-6 max-w-2xl text-sm text-fd-muted-foreground">
						Today you deploy a pre-built workload from a spec. Next, push your
						source and let Overdrive build, release, preview, and roll back.
					</p>
					<div className="grid gap-px overflow-hidden border border-dashed border-fd-border bg-fd-border sm:grid-cols-2 lg:grid-cols-4">
						{roadmap.map((r) => (
							<div key={r.title} className="bg-fd-background p-6">
								<span className="mb-3 inline-block border border-fd-border px-2 py-0.5 font-mono text-[10px] uppercase tracking-widest text-fd-muted-foreground">
									Planned
								</span>
								<h4 className="mb-2 text-base font-semibold">{r.title}</h4>
								<p className="text-sm text-fd-muted-foreground">{r.body}</p>
							</div>
						))}
					</div>

					{/* Self-healing */}
					<h3 className="mb-1 mt-12 text-lg font-semibold">
						Self-healing — tiered, ending in an SRE agent
					</h3>
					<p className="mb-6 max-w-2xl text-sm text-fd-muted-foreground">
						Restarting a failed instance is the easy tier. The intended end
						state investigates: correlate across the fleet by workload
						identity, find the root cause, and apply typed fixes through a
						risk-based approval gate — auto-applying the safe ones, asking
						before the risky ones, and remembering every resolved incident.
					</p>
					<div className="grid gap-px overflow-hidden border border-dashed border-fd-border bg-fd-border sm:grid-cols-2 lg:grid-cols-3">
						{selfHealingTiers.map((t) => (
							<div key={t.title} className="bg-fd-background p-6">
								<span className="mb-3 inline-block border border-fd-border px-2 py-0.5 font-mono text-[10px] uppercase tracking-widest text-fd-muted-foreground">
									Planned
								</span>
								<h4 className="mb-2 text-base font-semibold">{t.title}</h4>
								<p className="text-sm text-fd-muted-foreground">{t.body}</p>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* Honest status + CTA */}
			<section>
				<div className="container mx-auto max-w-5xl px-4 py-20">
					<p className="mb-3 font-mono text-xs uppercase tracking-widest text-[var(--color-brand)]">
						{"// Where it is today"}
					</p>
					<h2 className="mb-3 font-serif text-3xl font-normal md:text-4xl">
						Early, single-node, and honest about it.
					</h2>
					<p className="mb-8 max-w-2xl text-fd-muted-foreground">
						Overdrive runs on a single node today and is pre-production. The
						docs mark every behaviour still at the design stage rather than
						describing it as shipped, and there are no benchmark numbers on
						this page because there is nothing at fleet scale to measure yet.
						It is source-available, and converts to a permissive open-source
						licence two years after each release. If you are weighing it
						against Kubernetes, Nomad, or Fly.io, the comparison page makes
						the case and the counter-case plainly.
					</p>
					<div className="flex flex-wrap gap-4">
						<Link
							href="/docs/how-to/deploy-a-workload"
							className="rounded-md bg-fd-primary px-6 py-3 font-semibold text-fd-primary-foreground transition-all hover:bg-[var(--color-brand-2)] hover:shadow-[0_0_32px_rgba(255,92,40,0.35)]"
						>
							Deploy a workload
						</Link>
						<Link
							href="/docs/comparisons"
							className="rounded-md border border-fd-border px-6 py-3 font-medium transition-colors hover:bg-fd-muted"
						>
							See how it compares
						</Link>
					</div>
				</div>
			</section>
		</main>
	);
}
