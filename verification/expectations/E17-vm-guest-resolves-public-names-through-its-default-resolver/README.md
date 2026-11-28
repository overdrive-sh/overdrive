# E17 — a VM guest resolves the herdr dev box's public names through its default resolver, and a nonexistent name fails promptly and definitely

**Surface:** E (end-to-end through a real deployment) · **KPI:** K-GPD-1, K-GPD-2 (feature delta) · **Status:** `pending` (claim authored in DISTILL; not captured)

<!-- Status rationale: authored 2026-11-27 before any implementation evidence exists. Do not set `satisfied` from the authoring or implementing agent; a different reviewer judges the captured evidence. -->

## Expectation

Ana Moreno deploys her herdr dev box, an ordinary VM workload, with
`overdrive serve` and `overdrive deploy herdr-box.toml` on the metal host. She
installs nothing in the guest: it keeps its single nameserver, the guest gateway
`100.95.0.1`. Programs inside it call `getaddrinfo` and connect, as they would on
any host.

On the delivered product, one recorded run shows three things, each judged from
the captured text:

1. **The dev box's real lookup targets resolve and are reachable.** From
   inside the guest, `api.anthropic.com`, `github.com`, `crates.io` and
   `herdr.dev` (the host herdr itself installs and updates from) each return
   at least one IPv4 address through `getaddrinfo`, and a bare TCP connection
   (no protocol spoken) to one resolved IPv4 address on port 443 completes.
   Each lookup's and each connect's elapsed time is recorded next to its
   result; none of the lookups waits for the guest resolver's 5 second timeout
   (the pre-feature failure was `EAI_AGAIN` after 5005 ms, metal run
   b20bcb2fb8028a614e91755c at fe0c3adcf). The capture opens that connection
   with the guest kit's `resolve-tcp-connect` program (user ruling, 2026-11-28);
   it speaks no protocol and closes the connection at once.
2. **A nonexistent public name gets a definite, prompt "Name does not resolve".**
   `no-such-host-7f3a.example.com` fails with `EAI_NONAME`, not `EAI_AGAIN`,
   in well under the guest resolver's 5 second timeout.
3. **Nothing was hand-installed.** The guest's `/etc/resolv.conf` still names only
   `100.95.0.1`; no resolver, route or rule was added to the guest or the host by
   the capture; the node's upstream is the host's own resolver file.

The V-4 context receipts (the guest-observed time of one lookup whose upstream
answer lies in an Overdrive-owned range, answered by a controlled upstream
returning `10.98.0.7`, and of one lookup with the host's upstream resolvers
unreachable) are not part of this capture: both need the host's resolver file
or its upstream reachability changed, which the capture's third claim forbids,
and a controlled DNS upstream is test tooling that this black-box capture does
not carry. Their correctness is held by the test tiers
(`crates/overdrive-cli/tests/integration/guest_public_dns.rs`).

What would refute it: any of the four names failing to resolve or taking about
5 s, a TCP connection to a resolved address on 443 failing or timing out, the
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
`overdrive deploy` of `herdr-box.toml`, a finite VM Job (the lookups do not depend on the workload kind) whose guest program runs
`overdrive-guest-peer resolve-tcp-connect 443 <names>...` (the guest-kit
program; the captured console is the record), recording verbatim console lines,
each lookup's and connect's elapsed milliseconds, the guest's `resolv.conf`, `uname -r`, the SHA and
the dirty state. A point-in-time capture of four named lookups with a connect each, and one
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
