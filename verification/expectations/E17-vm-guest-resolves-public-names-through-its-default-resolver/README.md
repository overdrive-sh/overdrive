# E17 — a VM guest resolves the herdr dev box's public names through its default resolver, and a nonexistent name fails promptly and definitely

**Surface:** E (end-to-end through a real deployment) · **KPI:** K-GPD-1, K-GPD-2 (feature delta) · **Status:** `pending` (claim authored in DISTILL; not captured)

<!-- Status rationale: authored 2026-11-27 before any implementation evidence exists. Do not set `satisfied` from the authoring or implementing agent; a different reviewer judges the captured evidence. -->

## Expectation

Ana Moreno deploys her herdr dev box, an ordinary VM Service, with
`overdrive serve` and `overdrive deploy herdr-box.toml` on the metal host. She
installs nothing in the guest: it keeps its single nameserver, the guest gateway
`100.95.0.1`. Programs inside it call `getaddrinfo` and connect, as they would on
any host.

On the delivered product, one recorded run shows three things, each judged from
the captured text:

1. **The dev box's real lookup targets resolve.** From inside the guest,
   `api.anthropic.com`, `github.com`, `crates.io` and `herdr.dev` (the host
   herdr itself installs and updates from) each return at least one IPv4
   address through `getaddrinfo`. Each lookup's elapsed time is recorded next to
   its result; none of them waits for the guest resolver's 5 second timeout
   (the pre-feature failure was `EAI_AGAIN` after 5005 ms, metal run
   b20bcb2fb8028a614e91755c at fe0c3adcf). That a resolved address is
   *reachable* is not part of this claim: the guest kit's `resolve-connect`
   program speaks the test peer's byte exchange and cannot connect to a real
   host on 443. Reachability through the guest's egress is held by the
   walking-skeleton test (G-1). Whether this capture also opens a bare TCP
   connection to a resolved address on 443 is decided with the capture command
   at DELIVER; if it does, it needs a guest program that connects without
   speaking a protocol, which the guest kit does not have today.
2. **A nonexistent public name gets a definite, prompt "Name does not resolve".**
   `no-such-host-7f3a.example.com` fails with `EAI_NONAME`, not `EAI_AGAIN`,
   in well under the guest resolver's 5 second timeout.
3. **Nothing was hand-installed.** The guest's `/etc/resolv.conf` still names only
   `100.95.0.1`; no resolver, route or rule was added to the guest or the host by
   the capture; the node's upstream is the host's own resolver file.

The capture also records, as context for the reviewer and without being a
claim of its own, the guest-observed time of one lookup whose upstream answer
lies in an Overdrive-owned range (a controlled upstream answering `10.98.0.7`),
and of one lookup with the host's upstream resolvers unreachable. These are
V-4 receipts; the correctness of both is held by the test tiers.

What would refute it: any of the four names failing or taking about 5 s, the
nonexistent name failing with `EAI_AGAIN`, a changed guest `resolv.conf`, or
evidence that the capture installed a resolver, route or rule.

- Anchor: docs/feature/guest-public-dns-resolution/feature-delta.md AC-1, AC-3, AC-4 (DISCUSS US-GPD-1)
- Anchor: docs/feature/guest-public-dns-resolution/feature-delta.md K-GPD-1, K-GPD-2, V-4
- Anchor: docs/product/architecture/adr-0356-the-node-dns-responder-forwards-gateway-addressed-non-mesh-queries-to-the-host-resolvers.md
- Anchor: docs/product/journeys/run-a-vm-workload.yaml step 7

## Verification

Evidence is captured at DELIVER against the built `overdrive` binary on the
native-metal substrate (`.claude/rules/testing.md` § "Native-metal expectation
captures run with `--exclusive`"): one `overdrive serve`, one
`overdrive deploy` of a VM Service whose guest program runs
`overdrive-guest-peer resolve` lookups (the guest-kit program; the captured
console is the record; the capture command is decided at DELIVER), recording verbatim console lines, each
lookup's elapsed milliseconds, the guest's `resolv.conf`, `uname -r`, the SHA and
the dirty state. A point-in-time capture of four named lookups and one
nonexistent name, not a matrix and not a latency distribution; the forwarder's
contract is held by the test tiers
(`crates/overdrive-control-plane/tests/acceptance/dns_forwarder.rs`,
`.../integration/dns_forwarding_real_wire.rs`,
`crates/overdrive-cli/tests/integration/guest_public_dns.rs`). The four real
names require the metal host's own resolver to reach the internet. A reviewer
who did not write the implementation judges the claims from the captured text and
sets the status.

## Evidence

Not captured. Status `pending` until the DELIVER capture is reviewed.
