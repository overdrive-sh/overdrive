import Link from "next/link";

// Slice 08 (US-08, J-DOCS-001) — the marketing landing page at `/`, rendered
// inside the existing `app/(home)/layout.tsx` (`HomeLayout` + `baseOptions()`)
// so `/`, `/docs`, and `/blog` all share ONE nav shell (the baseOptions_shell
// invariant). Content is PORTED from the repo-root `index.html` — the approved
// marketing copy (C-6: real content only, no invented features). This is a
// placeholder content-port, NOT a pixel-perfect redesign; it carries the value
// prop and a clear path into the docs.
//
// Statically rendered (no "use client", no client-only hooks) so the hero value
// prop is in the server-rendered HTML on first load — the DISCUSS DoR 3rd UAT
// cold-load scenario for US-08.

export const metadata = {
	title: "Overdrive — Orchestration infrastructure for a new generation of workloads",
	description:
		"One Rust binary. Kernel-native dataplane. Built on the primitives Kubernetes never had.",
};

const valueProps = [
	{
		audience: "Platform Engineering",
		title: "Get out from under the stack you maintain.",
		body: "Stop running etcd, cert-manager, an ingress controller, a service mesh, and a CNI plugin as four independent failure domains. Overdrive ships them as one binary, with three-node HA that fits in 80 MB of memory.",
	},
	{
		audience: "SRE & On-Call",
		title: "Incidents that investigate themselves.",
		body: "Every eBPF event carries cryptographic workload identity. The native SRE agent correlates across alerts via SQL joins, attaches signed BPF probes to verify hypotheses, and proposes typed remediations through a graduated approval gate.",
	},
	{
		audience: "AI Engineering",
		title: "Agents that can't exfiltrate what they don't have.",
		body: "Prompt injection becomes structurally inert. The credential proxy holds the real keys. Domain allowlists run in-kernel via TC eBPF. BPF LSM blocks raw sockets. Security is enforced by infrastructure, not by the model's judgment.",
	},
] as const;

const metrics = [
	{ figure: "~10×", label: "Less control-plane RAM", note: "~100 MB vs ~1 GB on Kubernetes" },
	{ figure: "~100×", label: "Less mTLS CPU overhead", note: "kTLS in-kernel vs Envoy sidecar" },
	{ figure: "~50×", label: "Faster scheduling", note: "< 100 ms vs 1–10 s on Kubernetes" },
	{ figure: "2.3×", label: "Workload density", note: "~70% utilization vs ~30% baseline" },
	{ figure: "<10 s", label: "Node join", note: "vs 2–5 minutes on Kubernetes" },
	{ figure: "~1 ms", label: "WASM cold start", note: "Wasmtime warm pool, no Firecracker tax" },
] as const;

const pillars = [
	{
		index: "01 / Own your primitives",
		title: "Every dependency is a future incident.",
		body: "No etcd. No Envoy. No SPIRE. No CNI. Every critical subsystem is built into the platform or is a standard Rust library. External processes you didn't write are operational liabilities — they get cut.",
	},
	{
		index: "02 / The kernel is the dataplane",
		title: "Userspace proxies become unnecessary.",
		body: "Service routing, network policy, load balancing, mTLS, and telemetry happen at line rate in the kernel via aya-rs. No sidecar tax. No proxy reconfigurations. No tail-latency spikes from a userspace hop.",
	},
	{
		index: "03 / Security is structural",
		title: "mTLS isn't an option you remember to enable.",
		body: "Every connection is wrapped in kTLS with a SPIFFE identity. Policy is enforced in-kernel by BPF LSM. A compromised workload, a misconfigured pod, and a malicious dependency all hit the same walls.",
	},
] as const;

export default function HomePage() {
	return (
		<main className="flex flex-1 flex-col">
			{/* Hero */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-5xl px-4 py-20 md:py-28">
					<p className="mb-4 font-mono text-xs uppercase tracking-widest text-fd-muted-foreground">
						Source-available · FSL-1.1-ALv2 · Apache 2.0 after 2 years
					</p>
					<h1 className="max-w-3xl text-4xl font-bold leading-tight tracking-tight md:text-6xl">
						Redefining how compute runs.
					</h1>
					<p className="mt-6 max-w-2xl text-lg text-fd-muted-foreground md:text-xl">
						Orchestration infrastructure for a new generation of workloads.
						One Rust binary. Kernel-native dataplane. Built on the primitives
						Kubernetes never had.
					</p>
					<div className="mt-8 flex flex-wrap gap-4">
						<Link
							href="/docs"
							className="rounded-md bg-fd-primary px-6 py-3 font-medium text-fd-primary-foreground transition-opacity hover:opacity-90"
						>
							Read the docs
						</Link>
						<Link
							href="/blog"
							className="rounded-md border border-fd-border px-6 py-3 font-medium transition-colors hover:bg-fd-muted"
						>
							Read the blog
						</Link>
					</div>
					<dl className="mt-12 flex flex-wrap gap-x-10 gap-y-4 font-mono text-sm text-fd-muted-foreground">
						<div>
							<dt className="sr-only">Node image</dt>
							<dd>~50 MB node image</dd>
						</div>
						<div>
							<dt className="sr-only">Control plane</dt>
							<dd>~30 MB single-mode control plane</dd>
						</div>
						<div>
							<dt className="sr-only">Node join</dt>
							<dd>&lt; 10s node join</dd>
						</div>
					</dl>
				</div>
			</section>

			{/* Value props by audience */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto grid max-w-5xl gap-8 px-4 py-16 md:grid-cols-3">
					{valueProps.map((vp) => (
						<div key={vp.audience}>
							<p className="mb-2 font-mono text-xs uppercase tracking-widest text-fd-muted-foreground">
								{vp.audience}
							</p>
							<h3 className="mb-3 text-xl font-semibold">{vp.title}</h3>
							<p className="text-fd-muted-foreground">{vp.body}</p>
						</div>
					))}
				</div>
			</section>

			{/* By the numbers */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-5xl px-4 py-16">
					<h2 className="mb-2 text-2xl font-bold md:text-3xl">
						Architecture decisions, measured at fleet scale.
					</h2>
					<p className="mb-10 max-w-2xl text-fd-muted-foreground">
						Not micro-optimizations. These are direct consequences of the
						design — the kind of differences that turn three racks back into
						one.
					</p>
					<div className="grid gap-8 sm:grid-cols-2 md:grid-cols-3">
						{metrics.map((m) => (
							<div key={m.label} className="border-l border-fd-border pl-4">
								<div className="font-mono text-3xl font-bold">{m.figure}</div>
								<div className="mt-1 font-medium">{m.label}</div>
								<div className="text-sm text-fd-muted-foreground">{m.note}</div>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* Why now */}
			<section className="border-b border-fd-border">
				<div className="container mx-auto max-w-5xl px-4 py-16">
					<h2 className="mb-2 text-2xl font-bold md:text-3xl">
						Kubernetes was right for 2014. It is not right for 2026.
					</h2>
					<p className="mb-10 max-w-2xl text-fd-muted-foreground">
						Stable eBPF APIs, kernel TLS offload, production Rust systems
						libraries, and embeddable WASM only matured in the last two years.
						Overdrive is the orchestrator that becomes possible when all four
						exist at once.
					</p>
					<div className="grid gap-8 md:grid-cols-3">
						{pillars.map((p) => (
							<div key={p.index}>
								<p className="mb-2 font-mono text-xs uppercase tracking-widest text-fd-muted-foreground">
									{p.index}
								</p>
								<h3 className="mb-3 text-lg font-semibold">{p.title}</h3>
								<p className="text-fd-muted-foreground">{p.body}</p>
							</div>
						))}
					</div>
				</div>
			</section>

			{/* CTA */}
			<section>
				<div className="container mx-auto max-w-5xl px-4 py-20 text-center">
					<h2 className="text-2xl font-bold md:text-3xl">
						One binary. Every workload type.
					</h2>
					<p className="mx-auto mt-4 max-w-xl text-fd-muted-foreground">
						Built on the primitives Kubernetes never had. Start with the
						architecture and the design decisions in the docs.
					</p>
					<div className="mt-8 flex flex-wrap justify-center gap-4">
						<Link
							href="/docs"
							className="rounded-md bg-fd-primary px-6 py-3 font-medium text-fd-primary-foreground transition-opacity hover:opacity-90"
						>
							Get started in the docs
						</Link>
					</div>
				</div>
			</section>
		</main>
	);
}
