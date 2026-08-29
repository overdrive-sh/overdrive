# E08 — VM guest boot failure is truthful and clean; exit 78 remains ordinary

**Surface:** E (end-to-end) · **KPI:** Q7 · **Status:** `pending`

## Expectation

Through the built operator binary, a VM Job whose deploy-selected custom rootfs
makes the production resolver write fail is reported as a pre-READY boot
failure, never executes its command, consumes no restart attempt, and leaves no
allocation residue. The complement is equally load-bearing: after successful
READY/EXEC, an operator command that exits 78 is reported as an ordinary Job
result rather than a setup-failure sentinel.

- Anchor: S-GTI-05, S-GTI-08a, and S-GTI-08b in
  `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
- Anchor: DESIGN Q7 in
  `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`
- Anchor: ADR-0088 and ADR-0089

## Verification contract

Use the same native, non-virtualized x86_64 KVM preflight and host-wide
120-second command-lifetime lease specified by E07. Nested KVM is rejected;
Lima is compile-only. Unknown or contradictory virtualization signals block.

The eventual command is:

```text
verification/harness/run-expectation.sh E08
  -> cargo xtask metal run -- <E08 native runner>
  -> built overdrive serve
  -> overdrive deploy <custom-rootfs-resolver-failure-spec>
  -> overdrive workload describe <failed-id>
  -> overdrive deploy <ready-then-exit-78-spec>
  -> overdrive workload describe <exit-78-id>
  -> overdrive job stop <id> for any remaining intent
```

Required evidence, all verbatim and commit-pinned:

1. **Command/state:** deploy and describe outputs plus a 90-second bounded poll
   trail. The resolver case shows one terminal Failed attempt, resolver-stage
   detail, exact available VMM exit code, and unchanged durable restart count;
   it never shows READY, Running, or operator EXEC. The complement proves READY
   precedes EXEC and describe reports ordinary exit code 78, never boot failure
   or an unreported guest exit. Total deadline is 240 seconds for both cases.
2. **Wire:** the resolver-failure allocation emits no guest-originated frame,
   guest `EXIT` frame, or operator marker; capture drop/truncation/ambiguity is
   failure. The exit-78 case may emit only traffic attributable after its READY
   and guard-live boundaries.
3. **Kernel:** target/sibling rule snapshots prove the failed allocation never
   leaves a rule behind and an independent running allocation's exact
   tag/handle/normalized program/counter/order is unchanged. Kernel reads use
   the strict D7 framing; a partial dump is no evidence.
4. **Cleanup:** within 30 seconds, no failed-allocation VMM, cgroup,
   rootfs clone, clone index, run directory, netns, tap, veth, route, nft rule,
   capture process/socket/fd, or custom-rootfs working copy remains. Console
   absent/empty/unreadable/open/read/mid-read errors may change only bounded
   detail selection; none may mask the original rejection or cleanup result.

## Evidence

None captured. This is a pending stub, not a narrated pass. Satisfaction needs
the native command and independent evidence-only review.
