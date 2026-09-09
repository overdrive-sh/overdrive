import Link from "next/link";

import { HoverImpeller } from "@/components/brand";

export const metadata = {
	title: "Overdrive — Everything you run, on one platform",
	description:
		"Deploy web services, background jobs, microVMs, unikernels, WebAssembly functions, and sandboxes on one platform — encrypted, load-balanced, and self-healing from the first deploy, on your own hardware.",
};

const workloadKinds = [
	"Services",
	"Jobs",
	"microVMs",
	"Unikernels",
	"WASM",
	"Sandboxes",
] as const;

const stats = [
	{ value: "6", label: "workload kinds, one control plane" },
	{ value: "1", label: "file to describe an app" },
	{ value: "0", label: "sidecars to inject" },
	{ value: "mTLS", label: "on by default, no certs to rotate by hand" },
] as const;

const capabilities = [
	{
		tag: "RUN",
		title: "Run anything",
		body: "Long-running services, batch jobs, microVMs, unikernels, WebAssembly functions, and sandboxes run side by side under one control plane — not a separate stack for the workloads that don't fit in a container.",
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
	},
] as const;

const platformCapabilities = [
	capabilities[3],
	capabilities[2],
	capabilities[1],
	capabilities[4],
	capabilities[5],
] as const;

const capabilityIconPaths = {
	RUN: "m12 3 9 5-9 5-9-5 9-5Z M3 12l9 5 9-5 M3 16l9 5 9-5",
	SECURITY: "M12 3 4 6v5c0 5 8 10 8 10s8-5 8-10V6l-8-3Z m-4 9 3 3 5-6",
	IDENTITY: "M5 3h14v18H5V3Z M9 9a3 3 0 1 0 6 0 3 3 0 0 0-6 0 M8 17c0-4 8-4 8 0",
	NETWORKING: "M9 3h6v6H9V3Z M2 15h6v6H2v-6Z M16 15h6v6h-6v-6Z M12 9v3 M5 15v-3h14v3",
	RELIABILITY: "M2 12h4l3-8 6 16 3-8h4",
	OBSERVABILITY: "M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12Z M9 12a3 3 0 1 0 6 0 3 3 0 0 0-6 0",
	SECRETS: "M14 6a5 5 0 1 0 0 10 5 5 0 0 0 0-10Z M10 15l-7 7H1v-4l7-7 M15 10h.01",
	DEPLOY: "M3 4h18v16H3V4Z m4 5 3 3-3 3 m6 0h4",
	INFRASTRUCTURE: "M3 3h18v7H3V3Z M3 14h18v7H3v-7Z M7 6.5h.01 M7 17.5h.01 M12 6.5h5 M12 17.5h5",
} as const;

function CapabilityIcon({ kind }: { kind: keyof typeof capabilityIconPaths }) {
	return (
		<svg
			className="od-capability-icon"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			strokeWidth="1.6"
			strokeLinecap="round"
			strokeLinejoin="round"
			aria-hidden="true"
			focusable="false"
		>
			<path d={capabilityIconPaths[kind]} />
		</svg>
	);
}

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
		a: "Long-running services, batch jobs, microVMs, unikernels, WebAssembly functions, and sandboxes — six workload kinds under one control plane, described in one spec format.",
	},
	{
		q: "How does it compare to Kubernetes, Nomad, or Fly.io?",
		a: "The comparison page makes the case and the counter-case plainly, including where Overdrive is the wrong choice today.",
	},
] as const;

const deploymentSpec = `[service]
id       = "payments"
replicas = 1

[exec]
command = "/opt/payments/bin/server"

[[listener]]
port = 8080

[[health_check.readiness]]
type = "http"
path = "/healthz"
port = 8080`;

export default function HomePage() {
	return (
		<div className="od-landing">
			<section className="od-hero">
				<div className="od-hero-kinds" aria-label="Workload kinds">
					{workloadKinds.map((kind) => (
						<span key={kind}>{kind}</span>
					))}
				</div>

				<div className="od-hero-copy">
					<h1 className="od-hero-title">
						Everything you run, <em>on one platform.</em>
					</h1>
					<p className="od-hero-description">
						Deploy long-running services, batch jobs, microVMs, unikernels,
						WebAssembly functions, and sandboxes — with mutual TLS, load balancing, and self-healing built in.
						One platform to operate on your own hardware, instead of a stack you
						assemble and babysit.
					</p>
					<div className="od-actions">
						<Link className="od-button" href="/docs/how-to/deploy-a-workload">
							Deploy a workload
						</Link>
						<Link className="od-button-secondary" href="/docs">
							Read the docs
						</Link>
					</div>
				</div>
			</section>

			<section id="platform" className="od-platform">
				<div className="od-section-heading">
					<p className="od-eyebrow">One platform</p>
					<h2>Included, not assembled.</h2>
					<p>
						The hard parts of running an app — encryption, identity, load balancing,
						secrets, telemetry — are usually six products you pick, wire together,
						and keep in sync. Overdrive ships them as one platform, on by default.
						There is nothing to bolt on before the first deploy.
					</p>
					<Link className="od-inline-link" href="/docs/comparisons">
						See how it compares →
					</Link>
				</div>

				<div className="od-platform-board">
					<div className="od-board-caption">
						<span>Overdrive platform</span>
						<span>6 workload kinds, one control plane</span>
					</div>

					<div className="od-platform-map">
						<svg
							className="od-map-wires"
							viewBox="0 0 1200 450"
							preserveAspectRatio="none"
							aria-hidden="true"
						>
							<path d="M 200 62 C 360 62 400 225 600 225" />
							<path d="M 200 127 C 360 127 420 225 600 225" />
							<path d="M 200 192 C 360 192 430 225 600 225" />
							<path d="M 200 258 C 360 258 430 225 600 225" />
							<path d="M 200 323 C 360 323 420 225 600 225" />
							<path d="M 200 388 C 360 388 400 225 600 225" />
							<path d="M 600 225 C 800 225 840 92 1000 92" />
							<path d="M 600 225 C 800 225 840 158 1000 158" />
							<path d="M 600 225 C 800 225 840 225 1000 225" />
							<path d="M 600 225 C 800 225 840 292 1000 292" />
							<path d="M 600 225 C 800 225 840 358 1000 358" />
						</svg>

						<div className="od-map-column od-map-column-left">
							{workloadKinds.map((kind, index) => (
								<div className="od-map-node" key={kind}>
									<span className="od-map-node-index" aria-hidden="true">
										{String(index + 1).padStart(2, "0")}
									</span>
									<span>{kind}</span>
								</div>
							))}
						</div>

						<div className="od-map-engine" aria-label="Overdrive platform">
							<div className="od-engine-ring" aria-hidden="true" />
							<HoverImpeller className="od-platform-impeller" />
						</div>

						<div className="od-map-column od-map-column-right">
							{platformCapabilities.map((capability) => (
								<div className="od-map-node" key={capability.tag}>
									<span className="od-map-node-label">{capability.tag}</span>
									<span>{capability.title}</span>
								</div>
							))}
						</div>
					</div>

					<div className="od-board-foot">
						<span>Your hardware</span>
						<Link className="od-inline-link" href="/docs/concepts/architecture">
							Architecture →
						</Link>
					</div>
				</div>
			</section>

			<section className="od-stats" aria-label="Platform facts">
				<div className="od-stats-strip">
					{stats.map((stat) => (
						<div className="od-stat" key={stat.label}>
							<strong>{stat.value}</strong>
							<span>{stat.label}</span>
						</div>
					))}
				</div>
			</section>

			<section id="deploy" className="od-deploy">
				<div className="od-deploy-copy">
					<p className="od-eyebrow">Ship it</p>
					<h2>Ship from one file</h2>
					<p>
						Describe a workload in one TOML file — what to run, the CPU and memory
						it gets, and the health checks that tell the platform when it&apos;s ready.
						Deploy it with one command and watch it converge. No YAML templating, no
						apply-then-wait dance.
					</p>
					<ol className="od-deploy-steps">
						<li>
							<span>1</span>
							<p>
								<code>overdrive deploy</code> ships the spec and streams progress until
								your app is running and healthy.
							</p>
						</li>
						<li>
							<span>2</span>
							<p>
								Re-run it any time — deploy is idempotent, so an identical spec changes
								nothing and a CI job that can&apos;t tell whether it already landed is safe
								to run anyway.
							</p>
						</li>
					</ol>
					<Link className="od-inline-link" href="/docs/how-to/deploy-a-workload">
						Read the deploy guide →
					</Link>
				</div>

				<div className="od-terminal" aria-label="payments.toml deployment example">
					<div className="od-terminal-bar">
						<span className="od-terminal-file">payments.toml</span>
					</div>
					<pre>
						<code>{deploymentSpec}</code>
					</pre>
					<div className="od-terminal-command">
						<div>
							<span>$</span> overdrive deploy payments.toml
						</div>
						<div>payments · deploying…</div>
						<div>
							payments · <strong>running</strong>&nbsp;&nbsp;1/1 healthy
						</div>
					</div>
				</div>
			</section>

			<section id="capabilities" className="od-capabilities">
				<div className="od-section-heading">
					<p className="od-eyebrow">What you get</p>
					<h2>Run anything</h2>
					<p>
						Networking, security, identity, and reliability aren&apos;t add-ons you
						pick, wire together, and keep running. They&apos;re part of the platform — on
						by default, from the first deploy.
					</p>
				</div>

				<div className="od-capability-grid">
					{capabilities.map((capability) => (
						<article className="od-capability" key={capability.title}>
							<div className="od-capability-meta">
								<span className="od-capability-category">
									<CapabilityIcon kind={capability.tag} />
									<span className="od-capability-tag">{capability.tag}</span>
								</span>
							</div>
							<h3>{capability.title}</h3>
							<p>{capability.body}</p>
						</article>
					))}
				</div>
			</section>

			<section className="od-roadmap">
				<div className="od-roadmap-heading">
					<div className="od-roadmap-label">Planned</div>
					<p className="od-eyebrow">On the roadmap</p>
					<h2>From your source, not just a spec.</h2>
					<p>
						Push your source and Overdrive will build it, release it, preview every
						branch, and roll back in one command.
					</p>
				</div>
				<div className="od-roadmap-grid">
					{roadmap.map((item) => (
						<article className="od-roadmap-item" key={item.title}>
							<h3>{item.title}</h3>
							<p>{item.body}</p>
						</article>
					))}
				</div>
			</section>

			<section id="questions" className="od-faq">
				<div className="od-section-heading">
					<p className="od-eyebrow">Straight answers</p>
					<h2>Questions worth asking up front.</h2>
				</div>
				<div className="od-faq-list">
					{faqs.map((faq) => (
						<details className="od-faq-item" key={faq.q}>
							<summary>{faq.q}</summary>
							<p>{faq.a}</p>
						</details>
					))}
				</div>
			</section>

			<section className="od-final-cta">
				<div className="od-final-cta-copy">
					<p className="od-eyebrow">Where it is today</p>
					<h2>Early, single-node, and honest about it.</h2>
					<p>
						Overdrive runs on a single node today and is pre-production. The docs
						mark every behaviour still at the design stage rather than describing it
						as shipped, and there are no benchmark numbers on this page because
						there is nothing at fleet scale to measure yet. It is source-available,
						and converts to a permissive open-source licence two years after each
						release. If you are weighing it against Kubernetes, Nomad, or Fly.io, the
						comparison page makes the case and the counter-case plainly.
					</p>
					<div className="od-actions">
						<Link className="od-button" href="/docs/how-to/deploy-a-workload">
							Deploy a workload
						</Link>
						<Link className="od-button-secondary" href="/docs/comparisons">
							See how it compares
						</Link>
					</div>
				</div>
			</section>
		</div>
	);
}
