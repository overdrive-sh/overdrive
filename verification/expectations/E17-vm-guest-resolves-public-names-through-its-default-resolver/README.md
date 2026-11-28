# E17 — a VM guest resolves the herdr dev box's public names through its default resolver, and a nonexistent name fails promptly and definitely

**Surface:** E (end-to-end through a real deployment) · **KPI:** K-GPD-1, K-GPD-2 (feature delta) · **Status:** `pending` (re-captured 2026-11-28; review by a different agent due)

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
   `no-such-host-7f3a.invalid` (a name under `.invalid`, RFC 6761, which the
   host's upstream resolvers answer NXDOMAIN) fails with `EAI_NONAME`, not `EAI_AGAIN`,
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
nonexistent name failing with `EAI_AGAIN` or any code other than `EAI_NONAME`, a changed guest `resolv.conf`, or
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

### Re-capture, 2026-11-28 (current)

The first capture's claim 2 did not hold as written (below): the host's upstream
resolvers answer `no-such-host-7f3a.example.com` NOERROR with zero answers, the forwarder
relays that verbatim (correct by design), musl reports no data, and the guest program
printed `EAI_OTHER` because it named only NONAME, AGAIN and FAIL. Under the user's ruling
(2026-11-28) claim 2 was re-captured with `no-such-host-7f3a.invalid`, a name under
`.invalid` (RFC 6761), keeping the claim's meaning (a prompt, definite "Name does not
resolve" = `EAI_NONAME`). The guest program now also names `EAI_NODATA` and prints
`EAI_OTHER(<numeric code>)` for any other code (`d83f7d571`).

Before changing anything, a read-only query of the host's two upstream resolvers
(`151.115.125.3`, `151.115.125.11`, from the host's `/etc/resolv.conf`) for the new name
returned `status: NXDOMAIN` from both (metal run `5c2deec816c808b3436d2cdb`,
`evidence/diagnostic-host-upstream-answer-invalid-20261128.txt`).

Captured 2026-11-28 on the native-metal host (Linux 7.0.0-38-generic, `systemd-detect-virt`
reports `none`), SHA `d83f7d571995b670838664ecc6da9ebfa14719d7`, metal run
`074305492af4443a79355bd7` (`cargo xtask metal run --exclusive -- env OVERDRIVE_BPF_NATIVE=1
verification/harness/run-expectation.sh E17`). The source was clean
(`source sha=... dirty=false` in `capture.log`); the harness manifest says
`working_tree_dirty: true` only because the previous capture's renamed evidence files were
untracked in the checkout (`evidence/dirty-status.txt`; `dirty-diff.patch` is empty). Status
stays `pending`: a reviewer who did not capture it judges the text. Files are under
`evidence/` with the `20261128T123057Z` suffix (`capture.log` is append-only and holds all
captures; `run.log` and `verification.yaml` are this capture's, the earlier ones are kept
under their own timestamp suffixes).

The guest console, verbatim (`evidence/console-snapshots-20261128T123057Z/alloc-herdr-box-0.stream`):

```
resolve: nameserver=100.95.0.1
resolve: api.anthropic.com addr=160.79.104.10 ms=8 connect=ok port=443 connect_ms=1
resolve: github.com addr=140.82.121.4 ms=4 connect=ok port=443 connect_ms=0
resolve: crates.io addr=151.101.2.137 ms=28 connect=ok port=443 connect_ms=0
resolve: herdr.dev addr=104.21.36.201 ms=8 connect=ok port=443 connect_ms=0
resolve: no-such-host-7f3a.invalid error=EAI_NONAME ms=5
```

`overdrive workload describe herdr-box`: `Verdict: Succeeded`, attempt 1 `Terminated`, exit 0
(`evidence/describe-20261128T123057Z.txt`).

What the text shows, for the reviewer to judge:

- Claim 1: each of the four names resolved in 4 to 28 ms, and a bare TCP connection to the
  first resolved address on 443 completed in 0 to 1 ms.
- Claim 2: the nonexistent name failed with `EAI_NONAME` in 5 ms.
- Claim 3: the console shows `nameserver=100.95.0.1`, the first nameserver line of the
  guest's `/etc/resolv.conf` (the guest image carries no `cat`, so the whole file was not
  printed; the guest's init writes the single line `nameserver <gateway>`,
  `crates/overdrive-init/src/main.rs:590`). `capture.log` holds the host's `nft list
  ruleset`, `ip rule show`, `ip route show table all` and `/etc/resolv.conf` before and
  after this capture: the runner wrote none of them.
- V-4 context receipts: not captured, as before.

### First capture, 2026-11-28 (claim 2 did not hold as written; kept)

Captured 2026-11-28 earlier on the same host (Linux 7.0.0-38-generic, `systemd-detect-virt`
reports `none`), SHA `ce8a61cee9e3fbf490f4bd2e66220f60d3ab1eca`, metal run
`f2c7f64a5ead5ad09d8dea74` (`cargo xtask metal run --exclusive -- env OVERDRIVE_BPF_NATIVE=1
verification/harness/run-expectation.sh E17`). The runner's source was clean; the harness
manifest says `working_tree_dirty: true` only because the first attempt's evidence
(below) was untracked in the checkout. Status stays `pending`: a reviewer who did not
capture it judges the text below. Files are under `evidence/`.

The guest console, verbatim (`evidence/console-snapshots-20261128T114341Z/alloc-herdr-box-0.stream`):

```
resolve: nameserver=100.95.0.1
resolve: api.anthropic.com addr=160.79.104.10 ms=7 connect=ok port=443 connect_ms=1
resolve: github.com addr=140.82.121.4 ms=5 connect=ok port=443 connect_ms=0
resolve: crates.io addr=151.101.2.137 ms=36 connect=ok port=443 connect_ms=0
resolve: herdr.dev addr=104.21.36.201 ms=4 connect=ok port=443 connect_ms=0
resolve: no-such-host-7f3a.example.com error=EAI_OTHER ms=5
```

`overdrive workload describe herdr-box`: `Verdict: Succeeded`, attempt 1 `Terminated`, exit 0.

What the text shows, for the reviewer to judge:

- Claim 1: each of the four names resolved in 4 to 36 ms, and a bare TCP connection to the
  first resolved address on 443 completed in 0 to 1 ms.
- Claim 2: the nonexistent name failed in 5 ms, promptly and not as `EAI_AGAIN`, but the
  guest program printed `EAI_OTHER`, not `EAI_NONAME`: the program labels only `EAI_NONAME`,
  `EAI_AGAIN` and `EAI_FAIL` and prints every other resolver code as `EAI_OTHER`. A read-only
  query of the host's two upstream resolvers for this name returns `status: NOERROR` with
  zero answers (`evidence/diagnostic-host-upstream-answer-20261128.txt`), not NXDOMAIN, and
  musl `getaddrinfo` reports a NOERROR answer with no address as `EAI_NODATA`, which the
  program prints as `EAI_OTHER` (inferred from the upstream answer; the program does not
  print the numeric code). So the guest received the upstream's own definite answer
  promptly; the claim text as written (`EAI_NONAME`) is not what was observed for this
  name. A name under `.invalid` would give NXDOMAIN; the capture used the claim's name and
  was not altered.
- Claim 3: the guest console shows `nameserver=100.95.0.1`, the first nameserver line of
  the guest's `/etc/resolv.conf`, which the guest's init writes as the single line
  `nameserver <gateway>` (`crates/overdrive-init/src/main.rs:590`). The guest image carries
  no `cat`, so the whole file was not printed. `capture.log` holds the host's
  `nft list ruleset`, `ip rule show`, `ip route show table all` and `/etc/resolv.conf`
  before and after: the runner wrote none of them.
- V-4 context receipts: not captured. Both need the host's resolver file or its upstream
  reachability changed (and a controlled DNS upstream), which claim 3 forbids.

Earlier attempt, kept: run `8bf008b8e7f2ed199c855bf6` (SHA `7c46cb51c`) deployed the job
but the runner read the Job's `describe` as a Service table and polled until its
timeout, then lost the console (the VM run directory is removed when the allocation ends).
Its files are `run-20261128T113621Z.log`, `verification-20261128T113621Z.yaml`,
`describe-20261128T113621Z.txt` and the console-snapshots-20261128T113621Z directory; the runner
was fixed (console streamed from byte 0, Job attempt table) in `ce8a61cee`.
