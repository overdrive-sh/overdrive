# Intake — dial-by-name-responder

> **Raw input capture, NOT a wave artifact.** Source material for the
> DISCUSS wave. Authored by `/nw-new` on 2026-06-24. The DISCUSS agent
> reads this; it does not treat it as a deliverable.

## Source

GitHub issue **#243** — "In-agent node-local name responder for
dial-by-name (`svc.overdrive.local`)". Part of the
transparent-mtls-enrollment arc (**#236**, finalized in
`docs/evolution/2026-06-22-transparent-mtls-enrollment.md`). This is the
**deferred in-agent DNS responder slice** that the CLAUDE.md
vertical-slice precedent names explicitly ("the DNS responder daemon was
deferred… become later, independently-drivable slices").

## What the feature is

Answer `<job>.svc.overdrive.local` **inside the node agent** — a
name-answering listener in the same process that owns the agent-light L4
proxy and already holds the `ServiceBackendsResolve` index — so an
**unmodified** workload can dial a mesh peer **by name** and land at an
address `MtlsResolve` recognizes. Closes the dial-by-name path through
`overdrive serve` + `overdrive deploy`.

- **Headless path** (D-TME-10), distinct from #61's VIP path.
- **In the agent**, not a separate daemon, not in-kernel (D-TME-11).
- One listener serves every per-workload netns gateway address.

## Pinned contracts (already shipped / decided — do NOT re-litigate)

- **Injection — SHIPPED.** Each per-workload netns `/etc/resolv.conf`
  carries `nameserver <responder_addr>` via `veth_provisioner.rs`
  `WriteResolvConf`; `responder_addr` = the per-netns gateway
  (`plan.host_addr`, the Fly `fdaa::3` model — collision-free by
  construction). (D-TME-9)
- **Return shape — PINNED HEADLESS.** Return a `running-and-healthy` backend addr
  from `service_backends` — the **same** address `MtlsResolve.resolve`
  recognizes (one source, many readers, byte-consistent). No VIP, no
  #167. (D-TME-10)
- **Read mechanism — EXISTS.** `ServiceBackendsResolve` over an
  in-RAM, address-keyed, ownership-aware reverse index, built
  List-then-Watch + relist-on-`Lagged`. Already consumed by outbound
  resolve + inbound install. This slice makes it **"one source, THREE
  readers"** (outbound resolve + inbound install + **name answers**).
  (D-TME-11)

Today **nothing answers on `responder_addr`** — `getaddrinfo` reaches an
injected resolver with no responder behind it, so name resolution fails
in a deploy.

## Scope (in)

- In-agent name-answering listener on each per-netns gateway
  `responder_addr`, answering A/AAAA for `<job>.svc.overdrive.local`
  from `service_backends ∩ running-and-healthy` via a sibling name-keyed
  reader over the SAME service_backends rows (NOT the addr-keyed intercept
  index; refined by DESIGN ADR-0072)
  (K8s-headless / Fly `.internal` endpoint-set shape).
- Empty-candidate (no running-and-healthy backend) surfaced honestly (NXDOMAIN /
  empty answer) — **never** a stale address.
- Tier-3: a deployed workload's
  `getaddrinfo("<peer>.svc.overdrive.local")` resolves to a running-and-healthy
  backend and the connection is then intercepted + mTLS'd end-to-end
  through `serve` + `deploy`.

## Out of scope

- The **VIP path** (`<job>.svc.overdrive.local → fdc2::/16` VIP + XDP
  `SERVICE_MAP`) — that is **#61** (depends on #167).
- Backend addressing / inbound install — **#241**.

## The load-bearing unvalidated mechanism (→ SPIKE after DISCUSS)

**How does a single in-agent listener answer DNS queries sent to N
different per-netns gateway addresses?** A query is emitted from inside
each workload netns toward that netns' gateway addr; one host-side
listener must receive and answer all of them. This is a real-kernel
netns/routing/binding question with **no Tier-2 backstop** (`spike.md`
"no synthetic harness" case) — validate it in a timeboxed probe before
the walking skeleton. The arc's own precedent spiked throughout
(increment-a/b/i).

## Grounded code locations

- `crates/overdrive-control-plane/src/veth_provisioner.rs` —
  `WriteResolvConf`, `responder_addr_for_slot` (injection, shipped).
- `crates/overdrive-worker/src/mtls_intercept.rs`,
  `mtls_intercept_worker.rs` — `MtlsResolve` consumers.
- `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs`,
  `crates/overdrive-core/src/traits/mod.rs` — `MtlsResolve` trait.
- `ServiceBackendsResolve` index — the shared resolve surface
  (`subscribe_all_events()`, `all_service_backends_rows()`).
- Whitepaper §11.

## Additional requirement — runnable ping-pong demo (user, 2026-06-24)

Ship an **`examples/dial-by-name-responder/`** demo proving dial-by-name
end-to-end: **two services that ping-pong by name**.

- Two specs: `examples/dial-by-name-responder/a.toml` and `b.toml`
  (service A + service B), each with `[service]` / `[exec]` /
  `[resources]` / `[[listener]]` — the schema the `overdrive deploy
  <SPEC>` handler accepts (see `examples/coinflip-as-service.toml`,
  `examples/dns-resolver.toml`).
- **A calls B by name** (`b.svc.overdrive.local`); **B calls A by name**
  (`a.svc.overdrive.local`) — each resolving through the in-agent
  responder this feature builds, then intercepted + mTLS'd.
- **Each call increments a counter and stamps a fresh date.**
- Ping cadence **≈ every 10 seconds**.
- `examples/` is flat today (no subdirs); this introduces the
  `examples/<feature>/` subdir convention.

**Design implications to resolve in DISCUSS/DESIGN/DELIVER:**

- The demo needs a small **ping-pong workload program** (resolve peer by
  name → HTTP/TCP call on a ~10s loop; on inbound call increment counter
  + set new date + reply). `command` must point at a **real on-disk
  binary** in the deploy env (existing examples use `/usr/bin/socat` or
  a `/tmp`-staged helper) — decide: tiny Rust bin staged into the VM,
  or a shell + `curl`/`socat` loop. No phantom paths.
- This demo IS the feature's **walking-skeleton / Tier-3 acceptance**
  made operator-runnable — it cannot run until the responder answers, so
  it is scoped **inside** this feature, not built standalone first.
- Graduate to `verification/expectations/` (EDD) as the operator-surface
  proof, per `.claude/rules/verification.md`.
