# Overdrive

**Everything you run, on one platform.**

Overdrive is a workload orchestration platform written in Rust. It is designed
to run long-running services, batch jobs, microVMs, and WebAssembly functions
under one control plane — with mutual TLS, load balancing, identity,
and health checks built in. One platform to operate on your own hardware,
instead of a stack you assemble and babysit.

> **Status: early.** Overdrive runs on a single node and is
> pre-production. This README marks what runs now and what is intended.
> Nothing here is presented as shipped that isn't, and there are no benchmark
> numbers because there is nothing at fleet scale to measure yet. The full
> design — most of which is still ahead of the implementation — is tracked in
> the roadmap below.

## What runs

Single-node and pre-production. This is the shipped surface — what you can
actually run right now:

- **Ship from one file.** Describe a service or job in a single TOML spec and
  deploy it with `overdrive deploy`. Deploy is idempotent on the spec's content
  hash — an identical spec is a no-op, so it is safe to run straight from CI.
- **Every workload is a microVM.** Each service or job boots as a
  [Cloud Hypervisor microVM](https://overdrive.sh/docs/concepts/microvms)
  with its own kernel, isolated by hardware virtualization. The hypervisor runs
  confined — non-root, seccomp- and Landlock-restricted — and the platform
  reports the guest's real exit code, so a VM that boots and fails is never
  counted as a success. You supply the kernel and rootfs.
- **Health-checked and restarted.** Readiness and liveness probes gate traffic
  and catch failures. By default, a Service becomes a backend only after the guest
  reports every listener the spec declares. A startup probe you declare replaces
  that check. An allocation that fails its liveness check
  restarts, and the platform holds the replica count you declared. Every
  reconciler is re-evaluated every 30 seconds, so a missed event cannot strand a
  workload. See [reconcilers](https://overdrive.sh/docs/concepts/reconcilers).
- **In-kernel load balancing.** Traffic to a service spreads across its healthy
  backends in the kernel, with no userspace proxy in that load-balancing path.
  See the [architecture overview](https://overdrive.sh/docs/concepts/architecture#networking-uses-separate-mechanisms-for-separate-boundaries).
- **An identity per workload.** Every workload gets a short-lived cryptographic
  identity (SPIFFE) from a built-in certificate authority, so policy can name
  what a service is rather than the IP it holds. All allocations of a
  workload share one SPIFFE ID, and each holds its own certificate and key. The
  [workload identity lifecycle](https://overdrive.sh/docs/concepts/workload-identity-lifecycle)
  holds each identity while its allocation runs.
- **Ordinary sockets in the guest.** A microVM has no network interface. Its
  TCP and UDP sockets are captured in the guest and carried to the host over
  vsock, so a VM needs no per-VM host namespace, veth, or bridge. See
  [guest networking](https://overdrive.sh/docs/concepts/guest-networking).
- **Transparent mTLS between workloads.** The node agent holds each workload's
  identity, runs the TLS 1.3 handshake for mesh TCP connections, and installs
  the session keys into kTLS. Workloads open ordinary sockets and hold no
  certificates or private keys. Mesh UDP is not encrypted, and delivering
  Service-VIP UDP to a VM backend is unbuilt (issue
  [#310: UDP service delivery to VM workloads](https://github.com/overdrive-sh/overdrive/issues/310)).
  See [transparent mTLS](https://overdrive.sh/docs/concepts/transparent-mtls).
- **Reach a service by name.** On a single node, workloads resolve
  [service names](https://overdrive.sh/docs/concepts/service-names) such as
  `payments.svc.overdrive.local` to a stable address. The node agent connects
  each call to a running, healthy copy without an application discovery client.
- **Public HTTPS ingress.** Start `serve` with the four gateway flags (listen
  address, certified-key id, certificate chain, private key) and deploy a Route
  document with `overdrive deploy`. The
  [gateway](https://overdrive.sh/docs/concepts/public-ingress-gateway)
  terminates TLS 1.3, routes by hostname and path to a VM Service, and relays
  the request over the mesh. It speaks HTTP/1.1 only, serves one Route per node,
  and reads certificates from files you provide. See
  [expose a service publicly](https://overdrive.sh/docs/how-to/expose-a-service-publicly).

WebAssembly functions, persistent volumes, multi-node HA, and the immutable OS
are all on the roadmap below — designed, not yet shipped. Issue
[#348: node-local persistent volume for VM workloads](https://github.com/overdrive-sh/overdrive/issues/348)
tracks persistent volumes.

## Deploy a workload

A workload is a TOML file: what to run, the CPU and memory it gets, and the
health checks that tell the platform when it's ready.

```toml
# payments.toml
[service]
id       = "payments"
replicas = 1

[vm]
command = "/opt/payments/bin/server"
args    = ["--port", "8080"]
kernel  = "/var/lib/overdrive/kernel"
rootfs  = "/var/lib/overdrive/rootfs.ext4"

[resources]
cpu    = "500m"
memory = "128MiB"

[[listener]]
port     = 8080
protocol = "tcp"

[[health_check.readiness]]
type = "http"
path = "/healthz"
port = 8080
```

```console
$ overdrive deploy payments.toml
payments · deploying…
payments · running  1/1 healthy
```

Re-run it any time — deploy is idempotent, so an identical spec changes nothing
and a CI job that can't tell whether it already landed is safe to run anyway.
The full walkthrough is in the
[deploy guide](https://overdrive.sh/docs/how-to/deploy-a-workload).

The `[vm]` block names the guest command plus the operator-supplied kernel and
rootfs. The guest is hardware-isolated and its real exit code is reported back. See
[microVMs](https://overdrive.sh/docs/concepts/microvms).

## Concepts

- [Architecture overview](https://overdrive.sh/docs/concepts/architecture) — how
  the single-node binary, control plane, worker, and dataplane fit together.
- [microVMs](https://overdrive.sh/docs/concepts/microvms) — boot a
  hardware-isolated Cloud Hypervisor workload with your kernel and rootfs.
- [The intent / observation boundary](https://overdrive.sh/docs/concepts/intent-observation)
  — keep declared state separate from what the node observes.
- [Reconcilers](https://overdrive.sh/docs/concepts/reconcilers) — repeatedly
  converge observed state toward declared intent.
- [Workflows](https://overdrive.sh/docs/concepts/workflows) — run durable,
  ordered operations that terminate.
- [Reconcilers and workflows](https://overdrive.sh/docs/concepts/reconcilers-and-workflows)
  — see how the two control-plane primitives cooperate.
- [Identity](https://overdrive.sh/docs/concepts/identity) — issue workload
  SPIFFE identities from the built-in certificate authority.
- [Workload identity lifecycle](https://overdrive.sh/docs/concepts/workload-identity-lifecycle)
  — hold each SVID while its allocation runs and drop it on stop.
- [Transparent mTLS](https://overdrive.sh/docs/concepts/transparent-mtls) — the
  node agent authenticates and encrypts mesh connections with TLS 1.3 and kTLS.
- [Guest networking](https://overdrive.sh/docs/concepts/guest-networking) — carry
  a microVM's ordinary sockets to the host over vsock.
- [Public ingress gateway](https://overdrive.sh/docs/concepts/public-ingress-gateway)
  — terminate public HTTPS and relay each request to a Service over the mesh.
- [Service Names](https://overdrive.sh/docs/concepts/service-names) — resolve a
  service name to a live backend without an application discovery client.
- [Deterministic Simulation Testing](https://overdrive.sh/docs/concepts/deterministic-simulation-testing)
  — replay control-plane behavior from a seed.
- [Quint modeling](https://overdrive.sh/docs/concepts/quint-modeling) — specify
  concurrent protocols and check them against the Rust implementation.
- [Benchmark methodology](https://overdrive.sh/docs/concepts/benchmark-methodology)
  — turn replicates into comparable numbers with a noise floor.

## Roadmap

The platform is built in phases, each tracked as a GitHub milestone. Phase 1 is
essentially complete (22 of 24 issues closed). Phase 2 is in progress (29 of 39).
Phases 3–7 are planned and mostly unstarted, though the Cloud Hypervisor microVM
driver (#42) and the public ingress gateway (#54) landed ahead of the rest of
their phases and run. Some Phase 2 work, including transparent mTLS and
the SVID lifecycle, already runs. The other items remain tracked work.
Issue numbers link the specifics.

### [Phase 2 — eBPF dataplane & identity](https://github.com/overdrive-sh/overdrive/milestone/2) · in progress

The shipped mesh path runs TLS 1.3 in the node agent and hands record processing
to kTLS (#26), and the workload SVID lifecycle issues, rotates, and drops
identities (#35). Moving TLS termination into the guest kernel is tracked in #303.
Other tracked work includes agentless flow and resource telemetry (#31, #32),
node enrollment (#36), and the real-kernel test harness (#29, #30).

### [Phase 3 — workload drivers & policy](https://github.com/overdrive-sh/overdrive/milestone/3)

Run more than microVMs: WebAssembly serverless functions with scale-to-zero
(#44) and shared volumes (#43). A dual policy engine compiles Regorus and WASM
policy down to in-kernel verdicts (#38, #45, #47), and node drain migrates
workloads off unhealthy nodes — the reactive tier of self-healing (#50).

### [Phase 4 — gateway, sidecars & deployments](https://github.com/overdrive-sh/overdrive/milestone/4)

The built-in gateway (#54) runs for HTTP/1.1. The remaining gateway work
adds HTTP/2, gRPC, and WebSocket (#55, #341), a middleware pipeline with rate
limiting, auth, and circuit breaking (#56), and certificates it issues itself
over ACME (#57). WASM sidecars add the credential proxy and content inspector (#51, #52)
behind an SDK (#53). And deployment strategies grow up: rolling deploys (#64),
canary promotion with SLO-based rollback (#65), multi-stage rollout workflows
(#62, #66), and scheduled cron jobs (#63, #166).

### [Phase 5 — HA, multi-node & the immutable OS](https://github.com/overdrive-sh/overdrive/milestone/5)

High availability through a Raft intent store and a gossiped observation store
(#67, #68), with zero-downtime single→HA migration (#70). Operator identity and
CLI auth (#80, #81). And the sealed appliance itself: the `meta-overdrive`
immutable node OS and Image Factory (#75, #76), with an optional WireGuard or
Tailscale mesh underlay (#77, #78).

### [Phase 6 — observability & self-healing](https://github.com/overdrive-sh/overdrive/milestone/6)

The tiered self-healing story lands here. An LLM SRE agent (#85) runs
first-class investigations (#86), proposes typed actions through a risk-based
approval gate (#88), correlates signals by workload identity (#89), and reasons
over the reflexive and reactive tiers (#91) — learning from incident memory
(#94). Plus right-sizing (#92), scale-to-zero (#93), and persistent stateful
microVMs (#96–#100).

### [Phase 7 — multi-region, SDKs & supply chain](https://github.com/overdrive-sh/overdrive/milestone/7)

Multi-region federation (#104–#107), language SDKs for functions and workflows
(#109, #110), unikernel drivers (#112), OpenTelemetry export (#111), in-place OS
upgrades with dm-verity + TPM attestation and Secure Boot (#117, #118), and
operator SSO via OIDC (#119).

The full tracker — 99 open issues across these milestones — lives at
[github.com/overdrive-sh/overdrive/issues](https://github.com/overdrive-sh/overdrive/issues).

## Documentation

- [Documentation site](https://overdrive.sh/docs) — concepts, how-tos, and the
  CLI reference.
- [Deploy a workload](https://overdrive.sh/docs/how-to/deploy-a-workload) — the
  end-to-end path.
- [Expose a service publicly](https://overdrive.sh/docs/how-to/expose-a-service-publicly)
  — serve a VM Service over HTTPS through the gateway.
- [How it compares](https://overdrive.sh/docs/comparisons) — Overdrive versus
  Kubernetes, Nomad, and Fly.io, with the counter-case stated plainly.

## License

Source-available under the Functional Source License (FSL-1.1-ALv2). Every
release converts to Apache 2.0 two years after publication, under the
irrevocable future grant written into the license. Internal use is
unrestricted. The two-year window only restricts offering a competing
commercial product. See [LICENSE](LICENSE) for the full text.
