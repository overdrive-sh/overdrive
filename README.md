# Overdrive

**The open-source developer platform.** Functions, durable objects, sandboxed agents, queues, cron, KV, per-workload SQL, and S3-compatible object storage — running on infrastructure you own. One Rust binary. Source-available under [FSL-1.1-ALv2](LICENSE); every release converts to Apache 2.0 two years after publication.

> Everything Cloudflare does. On infrastructure you own.

[**overdrive.sh**](https://overdrive.sh) · [Whitepaper](docs/whitepaper.md) · [Roadmap](docs/whitepaper.md#24-roadmap) · [Commercial model](docs/commercial.md)

---

## What it looks like

```typescript
// hello.ts
export default {
    async fetch(req: Request, env: Env): Promise<Response> {
        const count = Number(await env.KV.get("count") ?? "0") + 1;
        await env.KV.put("count", String(count));
        return Response.json({ count });
    }
};
```

```sh
$ overdrive deploy hello.ts
✓ Function deployed to spiffe://overdrive.local/fn/hello
✓ Public URL: https://hello.abc123.overdrive.dev
✓ Bindings: KV=prod-kv

$ curl https://hello.abc123.overdrive.dev
{"count": 1}
```

From your laptop, from one bare-metal box, from a managed cloud — same binary, same CLI, same primitive surface. No kubectl. No helm. No service mesh to wire up. No hyperscaler.

*(v1 developer-platform launch target; see [Status](#status) below.)*

---

## The primitive catalog

| Primitive | What it is |
|---|---|
| **Functions** | WASM serverless compute; ~1 ms cold start; `env.*` bindings per primitive |
| **Durable Objects** | Single-writer WASM actors with per-instance KV + SQL storage and a globally-unique addressable name |
| **Sandboxed Agents** | Persistent microVMs for AI coding agents (Claude Code, Cursor-style) and long-running autonomous workers |
| **Stateful VMs** | Same primitive — for Postgres, Redis, CI runners, Jupyter, remote dev environments |
| **Queues** | At-least-once pull-based messaging; consumer-group semantics; `overdrive-fs`-backed |
| **Cron / Schedule** | First-class scheduled jobs with explicit DST and bounded catchup policy |
| **Event Bus** | Topic pub/sub via Corrosion CRDT gossip; local-SQLite subscriptions |
| **KV** | Eventually-consistent key-value; sub-ms local reads |
| **D1-shape SQL** | Per-workload libSQL; addressable from other workloads by SPIFFE identity |
| **R2-shape Object Storage** | S3-compatible object storage via Garage |
| **Gateway** | Automatic public HTTPS (ACME), per-workload routing, request replay, middleware pipeline |

All primitives share one identity model (SPIFFE), one policy engine (Regorus + WASM), one dataplane (eBPF), one telemetry pipeline (DuckLake). One binary.

---

## Why Overdrive

- **Self-hostable.** Same binary runs on your laptop, a bare-metal box, or a multi-region fleet. No proprietary control plane.
- **Kernel-native.** eBPF for routing, load balancing, mTLS (via kTLS), and policy enforcement — zero-overhead; no Envoy / Istio / CNI plugin.
- **Structurally secure.** SPIFFE identity on every workload; BPF LSM enforces syscall policy in-kernel; the credential proxy holds real keys so compromised agents cannot exfiltrate them.
- **Real workload range.** WASM functions and Cloud-Hypervisor microVMs under one scheduler — not containers-only, not functions-only.
- **One binary.** Control plane, node agent, gateway, and built-in CA in a single Rust binary. Role declared at bootstrap.
- **Source-available with a future grant.** FSL-1.1-ALv2 today; Apache 2.0 on every release's two-year anniversary. No long-term enclosure.

For the design in depth, the trade-offs, and the prior art, see [the whitepaper](docs/whitepaper.md).

---

## Status

**Pre-alpha. v0.1 in active development.**

The Rust workspace is scaffolded; the kernel-matrix testing harness is being wired up; Phase 1 (Foundation) is in progress. The **v1 developer-platform launch** — the first release where `overdrive deploy function.ts` against a single-node box produces a working public URL with KV / DB / R2 / Queue / EventBus / Schedule / Durable Object bindings — is targeted for Phase 5 on the [roadmap](docs/whitepaper.md#24-roadmap).

If you're looking for something to run in production today, come back later. If you're looking for a design worth engaging with, start with the whitepaper.

---

## Build from source

```sh
# Requires Rust 1.85+ (rust-toolchain.toml pins the exact version)
git clone https://github.com/overdrive-sh/overdrive
cd overdrive
cargo build --release

./target/release/overdrive --help
```

The development environment uses a Lima VM for eBPF work — see [`infra/`](infra/) for setup. Conventions and hook configuration are defined in [`CLAUDE.md`](CLAUDE.md), [`.claude/rules/development.md`](.claude/rules/development.md), and [`.claude/rules/testing.md`](.claude/rules/testing.md).

---

## Documentation

| Doc | What it's for |
|---|---|
| [Whitepaper](docs/whitepaper.md) | Architecture, design principles, primitive specs, roadmap |
| [Commercial model](docs/commercial.md) | Self-hosted (free), managed cloud, enterprise licensing |
| [Research](docs/research/) | Evidence-backed design decisions and prior-art surveys |
| [Strategy research](docs/research/strategy/) | Positioning, demand signal, OSS-CF analysis |

---

## Contributing

Overdrive is in an opinionated early-design phase. The whitepaper is the source of truth for architectural direction; the research directory records evidence-backed decisions as they're made.

Issues and discussions are welcome now. Code contributions are most useful once Phase 1 (Foundation) stabilises — adding workload primitives, drivers, and bindings against an unsettled core is costly to undo. Track milestones at [github.com/overdrive-sh/overdrive/milestones](https://github.com/overdrive-sh/overdrive/milestones).

---

## License

**Server binary:** [FSL-1.1-ALv2](LICENSE) — source-available. Internal use is unrestricted. Commercial Use is restricted for two years per release; every release converts to Apache 2.0 on its second anniversary under the irrevocable future-grant written into the licence.

**Client-side SDK** (`overdrive-ff`, when released): Apache-2.0.

See [`docs/commercial.md`](docs/commercial.md) for the rationale — and for the three commercial pillars (cloud platform, enterprise self-hosted licensing, source-available flywheel) that fund continued development.

---

*Made in Copenhagen. EU-sovereign by construction.*
