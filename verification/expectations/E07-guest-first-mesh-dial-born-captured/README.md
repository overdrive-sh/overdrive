# E07 — a VM guest's first mesh dial is born captured and mTLS-protected

**Surface:** E (end-to-end) · **KPI:** Q9/D7 · **Status:** `pending`

## Expectation

A real VM Job deployed through the built operator binary is silent until its
allocation-specific egress guard is proven live, then resolves a mesh peer by
name, exchanges byte-distinct plaintext at the workload-facing legs, and has
that first flow exactly accounted by the unchanged production nft rule while
the external path carries TLS and no cleartext.

- Anchor: S-GTI-01 and S-GTI-02 in
  `docs/feature/guest-stack-transparent-mtls-intercept/distill/test-scenarios.md`
- Anchor: DESIGN D7 / Q9 in
  `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`
- Anchor: ADR-0088 and ADR-0089

## Verification contract

Runtime is **native, non-virtualized x86_64 KVM only**. Nested KVM and Lima are
not evidence; Lima may compile the gated test with `--no-run`. Before any
claim, capture `uname -m`, literal `none` from `systemd-detect-virt`, the absence of a
`hypervisor` CPU flag, the required hardware virtualization CPU extensions, an
openable `/dev/kvm` with API version 12 and a successful create-and-close VM-fd
probe, cgroup v2, Cloud Hypervisor, kernel, and rootfs. Missing, unknown,
contradictory, unusable, or virtualized results block the expectation.

The eventual command is:

```text
verification/harness/run-expectation.sh E07
  -> cargo xtask metal run -- <E07 native runner>
  -> built overdrive serve + overdrive deploy <vm-job-spec>
  -> overdrive workload describe <id>
  -> overdrive job stop <id>
```

Before any shared remote-tree mutation, the canonical metal writer boundary
must acquire `/run/lock/overdrive-metal-shared.lock` with a 120-second timeout.
The same lease covers every `MetalAction::Run`, `MetalAction::Sync`, and
supported direct bootstrap writer; raw or legacy writers that do not
participate are prohibited from the canonical `~/overdrive` tree. The helper
records holder PID, UTC start, action, expectation id, workspace, and commit
SHA, and acknowledges ownership before `rsync --delete`. For E07 Run, the same
descriptor stays held through sync, preflight, serve/deploy, all captures,
stop, cleanup, and final probes, including signal/error paths. A timeout reports
the current owner and aborts without mutation. E07 runtime evidence is invalid
until this roadmap/DEVOPS prerequisite is implemented at the canonical
Run/Sync/bootstrap boundary.

Required evidence, all verbatim and commit-pinned:

1. **Command:** built-binary serve/deploy/describe/stop argv, exit status, and
   bounded poll trail. Running must be observed within 90 seconds; the whole
   scenario has a 180-second deadline.
2. **State:** exact allocation id, slot, netns inode, tap/host-veth names and
   ifindices, guest MAC/address, C3 completion, capture-ready, VMM spawn, READY,
   D7 before-cut, EXEC release, first connect, and terminal ordering.
3. **Wire:** complete pre-guard all-EtherType zero-frame capture; exact first
   directional SYN; original destination at leg-F; TLS application-data in
   both directions; absence of both plaintext markers on the peer path; zero
   capture drops/truncation/ambiguity.
4. **Kernel:** strict complete generation-bracketed `GETRULE`/`GETGEN` logs,
   loss-free nft notification stream, exact tag/handle/normalized production
   program, stable before/after pairs, and checked packet plus validated IPv4
   `tot_len` equality. Any reset, replacement, mutation, wrap, loss, partial
   dump, competing eligible tuple, or ambiguity fails.
5. **Cleanup:** within 30 seconds after `overdrive job stop`, no target VMM,
   cgroup, clone/index, run directory, netns, tap, veth, route, nft rule,
   capture process/socket/fd, or private fixture remains. Delta probes must not
   remove pre-existing resources.

## Evidence

None captured. The runner is a pending stub and must not be marked satisfied
until the native command completes and a different-fox reviewer audits only
the captured evidence.
