# Evolution — A VM guest resolves public names through its default resolver

**Feature:** `guest-public-dns-resolution` (GH #382). **Finalized:** 2026-11-28.
**Scope:** the node's DNS responder at the guest gateway `100.95.0.1`: classification of query names, forwarding of public names to the host resolvers over UDP and TCP, the owned-range guard, bounded in-flight forwards, and the descriptor-limit raise that bounds them. Sixteen implementation steps.
**Waves run:** DISCUSS and DESIGN (user approval 2026-11-25), DISTILL (2026-11-27), DELIVER (2026-11-27 to 2026-11-28).
The [feature contract and wave history](../feature/guest-public-dns-resolution/feature-delta.md), [roadmap](../feature/guest-public-dns-resolution/deliver/roadmap.json) and [execution log](../feature/guest-public-dns-resolution/deliver/execution-log.json) remain the delivery records.

## Summary

A VM guest with one nameserver, the gateway, can now resolve and reach public names with nothing installed in the guest. The responder classifies each name once by labels. Mesh names are answered as before, and an unknown platform-zone name is answered NXDOMAIN at the gateway. A gateway-addressed public name is forwarded to the upstreams in the host resolver file, which is re-read on every call. Forwarding runs off the receive thread under permits derived from the descriptor limit, and the excess is shed at once. A forwarded reply that carries an Overdrive-owned address is emptied whole and marked blocked. An unanswered query gets a definite SERVFAIL, and failure and recovery episodes warn once each. The same service is offered on TCP/53 bound to the gateway. Before the feature, a guest lookup of `github.com` waited 5005 ms and failed with `EAI_AGAIN`.

## Context

The herdr dev box, an ordinary VM workload, could not resolve any public name through its default resolver. The responder answered mesh names only and dropped everything else without a reply, so every lookup ended at the guest resolver's 5 second timeout. The baseline run is metal run `b20bcb2fb8028a614e91755c` at `fe0c3adcf`.

## Key decisions

Decisions GPD-1 to GPD-8 were approved by the user on 2026-11-25 and are recorded in ADR-0356 to ADR-0363 in `docs/product/architecture/`. The architecture brief carries the addendum.

| Decision | Outcome | ADR |
|---|---|---|
| GPD-1 | The responder forwards gateway-addressed non-mesh queries to the host resolvers | 0356 |
| GPD-2 | Upstreams are read from the host resolver file on each forwarded query | 0358 |
| GPD-3 | A forwarded reply carrying an Overdrive-owned address is emptied whole and marked blocked | 0359 |
| GPD-4 | The responder also serves TCP/53 bound to the guest gateway | 0360 |
| GPD-5 | Every non-mesh record type, EDNS option and DNSSEC bit passes through the forwarder unchanged | 0361 |
| GPD-6 | In-flight forwards are bounded by the descriptor limit and the excess is shed at once | 0362 |
| GPD-7 | `overdrive serve` raises its soft descriptor limit once at the binary entry and injects the read-back value | 0363 |
| GPD-8 | Remaining decisions as recorded in the feature delta; ADR-0357 holds the neighbouring decision | 0357 |

The ADR-0360 amendment of 2026-11-28 (D1 to D5, user-approved) is described under "Defect found during finalization".

Scope limitations were approved on 2026-11-25 and cite their issues in the feature delta: #386 (an operator health condition for upstream failure) and #383 (no allowlist or policy on non-mesh lookups). #385 tracks permit fairness.

## What shipped

| Group | Steps | Landed |
|---|---|---|
| 01 Pure forwarding pieces | 01-01..01-07 | Owned ranges, upstream selection and permit arithmetic, relay, failover, owned-range guard, episode warns, permits and shedding |
| 02 Host adapters | 02-01, 02-02 | Resolver file read per call, upstream exchange over UDP and TCP |
| 03 Descriptor limit | 03-01, 03-02 | Raise at the binary and inject, derive the permit bound at boot and report it |
| 04 Responder | 04-01..04-03 | One name classifier and gateway gate, UDP forwarding, TCP/53 |
| 05 Walking skeleton | 05-01 | A VM guest resolves public names through `serve` and `deploy` |
| 06 Defect fix | 06-01 | The TCP listener serves pipelined queries until the client closes (`73aa25657`) |

Step commits run from `1865eaa6a` (owned ranges) through `ea353a194` (the guest scenarios through `serve` and `deploy`). Review fixes: `5377b4c64`, `f53fad102`, `f8e2ab424`, `636ab2236`, and the bench fix `6cd1e3865`.

## Work completed (execution log)

All 16 steps are `DONE`, and `des-verify-integrity docs/feature/guest-public-dns-resolution/deliver/` reports "All 16 steps have complete DES traces". The log has two points the reader should know:

- RED is logged `SKIPPED` for steps 01-07, 02-02, 03-01, 03-02 and 05-01.
- Step 04-03 logged RED `EXECUTED`, but the crafter ran the tests only after implementing. The RED was not observed before the code existed.

## Verification

- **Full workspace suite on metal: 5381 of 5384.** The bench DNS smoke failure was caused by this feature (the gateway was assigned after the responder started) and is fixed in `6cd1e3865`. The two remaining failures, `cloexec_lint_workspace` (the `dup2` in `launch_clone.rs`) and the `vm_lifecycle_completion` conformance test, reproduce identically on the base commit `d03a6c7f7` and predate the feature.
- **E17 is satisfied** ([`verification/expectations/E17-vm-guest-resolves-public-names-through-its-default-resolver/README.md`](../../verification/expectations/E17-vm-guest-resolves-public-names-through-its-default-resolver/README.md), review 2026-11-28, metal run `074305492af4443a79355bd7` at `d83f7d571`). The first capture's claim 2 failed: `example.com` answers NOERROR with no data at the upstream, so it is not an NXDOMAIN case. By user ruling it was re-captured with a `.invalid` name.
- **Descriptor check requested by the user.** With soft = hard = 1,048,576 on kernel 7.0.0-38, `ip link show` makes 12 close calls and `nft list tables` makes 15, and neither calls `close_range` (`spike-scratch/guest-public-dns-nofile/check.sh`).
- Mutation testing is disabled for this repository (CLAUDE.md), so no mutation gate ran.

## Reviews

An adversarial and a subtractive review ran after each roadmap phase, and the fixes are listed above. The documentation and blog post went through an independent documentarist review that approved after three rounds. The last round's low findings were applied in `a232c0c57`. No separate review artifact was written, and this paragraph is the verdict record.

## Defect found during finalization

glibc's stub resolver pipelines the A and AAAA queries over one TCP connection. The responder served one query per connection, so glibc received the A reply and then EOF where the AAAA reply belonged, and `getaddrinfo` failed outright. musl opens one connection per query and was unaffected, and a v4-only guest sends no AAAA. The spike is [`findings-tcp-one-query-per-connection.md`](../feature/guest-public-dns-resolution/spike/findings-tcp-one-query-per-connection.md). ADR-0360 was amended on 2026-11-28 (D1 to D5, user-approved). W-11 was revised to assert service until the client closes, and W-16 was added as the regression. The fix is `73aa25657`, and the probe re-run shows it FIXED (`0eb71362e`).

## Lessons

- **A scripted upstream can hide a resolver-library behaviour.** The scenarios passed with one query per TCP connection because the test clients sent one query. Only a real glibc lookup pipelines, and the defect surfaced during finalization, not in DISTILL.
- **A capture claim needs a name whose upstream answer is known.** `example.com` is NOERROR with no data, so it cannot witness a prompt NXDOMAIN. A `.invalid` name can.
- **A feature can break an adjacent smoke test.** The bench DNS smoke failed because the gateway was assigned after the responder started. The full-suite run found it.

## Known unrelated issues found

- `cloexec_lint_workspace` fails on `launch_clone.rs` `dup2`, and `vm_lifecycle_completion` fails in the conformance suite. Both reproduce on `d03a6c7f7`.
- `test:blog` fails on a docs-search top-hit assertion that reproduces at HEAD without these changes.

## Documentation changed

`website/content/docs/concepts/guest-networking.mdx`, `concepts/service-names.mdx`, `concepts/architecture.mdx`, and the post `website/content/blog/vm-guests-resolve-public-names.mdx`. Validation: `typecheck`, `lint`, `build` and `assert:one-index` pass.

## Scope limits and deferrals

No issue was created or changed by this finalization. Tracked limitations carry their issues in the feature delta (#383, #385, #386). Open follow-ups for the user:

- #385's body is stale: it says a 256 cap, and the design derives the bound from the descriptor limit.
- #259 does not record the resolver-file image requirement that the design attributes to it.
- K-GPD-1 names `example.com`, but E17 used `herdr.dev`.

## Permanent artifacts

- ADRs 0356 to 0363 in `docs/product/architecture/` (already in their permanent location, so nothing was copied), and the addendum in `docs/product/architecture/brief.md`.
- Code: `crates/overdrive-control-plane/src/dns_responder/`, `crates/overdrive-control-plane/src/guest_network.rs`, `crates/overdrive-cli/src/main.rs`, `crates/overdrive-core/src/id.rs`.
- Verification: `verification/expectations/E17-vm-guest-resolves-public-names-through-its-default-resolver/`.
- Spike probes: `spike-scratch/guest-public-dns-*`, kept until the implementation supersedes them.

## Finalization

- **Issues closed:** GH #382: closed <pending>.
- **Metal cleanup:** <pending>.
- **Website validation and documentation review:** as under "Documentation changed" and "Reviews".
- **Session files removed:** `.nwave/des/deliver-session.json` and `.nwave/des/des-task-active`, neither of which was tracked.
