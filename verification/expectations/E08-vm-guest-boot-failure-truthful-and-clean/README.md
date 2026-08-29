# E08 — VM guest boot failure is truthful and clean; exit 78 remains ordinary

**Surface:** E (end-to-end) · **KPI:** Q7 · **Status:** `pending`

## Expectation

Through the built operator binary, a fresh VM Job whose real production
outbound TPROXY append is rejected by a deterministic kernel-chain fixture is
terminal, never executes, and leaves no allocation residue. A second VM Job
whose deploy-selected custom rootfs makes the production resolver write fail is
reported as a pre-READY boot failure with the same execution/cleanup guarantees.
The complement is equally load-bearing: after successful READY/EXEC, an
operator command that exits 78 is reported as an ordinary Job result rather
than a setup-failure sentinel.

- Anchor: S-GTI-05, S-GTI-08a, and S-GTI-08b in
  `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
- Anchor: DESIGN Q7 in
  `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`
- Anchor: ADR-0088 and ADR-0089

## Verification contract

Use the same native, non-virtualized x86_64 KVM preflight and host-wide
120-second supervising-session lease specified by E07. It is acquired before
the launcher's shared `rsync --delete` and held across sync, preflight, every
subcase, evidence, fixture restoration, and final probes. Nested KVM is
rejected; Lima is compile-only. Unknown or contradictory virtualization
signals block.

The eventual command is:

```text
verification/harness/run-expectation.sh E08
  -> cargo xtask metal run -- <E08 native runner>
  -> snapshot the exact host nft/FIB baseline
  -> create the E08 regular-prerouting-chain kernel fixture
  -> built overdrive serve
  -> overdrive deploy <fresh-guard-install-failure-spec>
  -> overdrive workload describe <guard-failed-id>
  -> stop serve; restore the exact recorded kernel delta; prove restoration
  -> built overdrive serve
  -> overdrive deploy <independent-sibling-vm-job>
  -> overdrive deploy <custom-rootfs-resolver-failure-spec>
  -> overdrive workload describe <failed-id>
  -> overdrive deploy <ready-then-exit-78-spec>
  -> overdrive workload describe <exit-78-id>
  -> overdrive job stop <id> for the sibling and any remaining intent
```

The fresh-install failure uses no injection flag or test-owned success path.
Starting from a baseline in which the `ip overdrive-mtls` table is absent, the
runner creates that test-owned table and a **regular, hookless** chain named
`prerouting`, plus one foreign sentinel rule whose userdata cannot match the
`ovdmtls` allocation prefix. The production ensure observes the chain name as
already present, the production installer attempts its real TPROXY expression,
and the kernel's `nft_tproxy_validate` hook check deterministically rejects
TPROXY in the unreferenced hookless chain because its reachable-hook mask does
not include `NF_INET_PRE_ROUTING`. The recorded failing operation/errno is
evidence; the runner does not call an install error seam. The fixture delta includes the
test-owned table/chain/sentinel and any production-created shared exemption,
`output` chain, fwmark rule, or local route. After the allocation-scoped probes
and serve shutdown, teardown removes only objects absent from the saved
baseline, then requires byte-for-byte normalized nft and typed FIB equality to
that baseline. If the required clean baseline cannot be established without
touching pre-existing state, E08 blocks before fixture creation.

Required evidence, all verbatim and commit-pinned:

1. **Command/state:** fixture commands, production install error, deploy and
   describe outputs, and 90-second bounded poll trails. The fresh guard case
   reaches terminal Failed with the real kernel install detail, never releases
   EXEC, and never reaches Running. The resolver case shows one terminal Failed
   attempt, resolver-stage detail, exact available VMM exit code, and unchanged
   durable restart count; it never shows READY, Running, or operator EXEC. The
   complement proves READY precedes EXEC and EXIT and describe reports ordinary
   exit code 78, never boot failure or an unreported guest exit. Total deadline
   is 300 seconds for all three cases.
2. **Wire:** neither failure allocation emits a guest-originated frame, guest
   `EXIT` frame, operator marker, or peer-path cleartext; capture
   drop/truncation/ambiguity is failure. The exit-78 case may emit only traffic
   attributable after its READY and guard-live boundaries.
3. **Kernel:** strict complete snapshots prove the test-owned sentinel is
   unchanged, no allocation-scoped target rule survives the rejected fresh
   install, and final fixture teardown equals the recorded nft/FIB baseline.
   The resolver subcase additionally proves the ordered sequence of an
   independent running allocation's `(userdata, handle, normalized full
   program, packets, bytes)` snapshot is unchanged. Kernel reads use the strict
   D7 framing; a partial dump is no evidence.
4. **Cleanup:** within 30 seconds, no failed-allocation VMM, cgroup,
   rootfs clone, clone index, run directory, netns, tap, veth, route, nft rule,
   capture process/socket/fd, or custom-rootfs working copy remains for either
   failed allocation. Console
   absent/empty/unreadable/open/read/mid-read errors may change only bounded
   detail selection; none may mask the original rejection or cleanup result.
   Fixture restoration is separately delta-scoped and must not be confused
   with product cleanup.

## Evidence

None captured. This is a pending stub, not a narrated pass. Satisfaction needs
the native command and independent evidence-only review.
