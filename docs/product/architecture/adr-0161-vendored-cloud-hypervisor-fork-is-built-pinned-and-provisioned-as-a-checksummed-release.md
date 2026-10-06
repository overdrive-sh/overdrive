# ADR-0161 — The vendored Cloud Hypervisor fork is built from a pinned commit and provisioned as a checksum-verified release artifact

## Status

**Proposed — decision D21 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*.

It implements the provisioning half of ruling **D13 = (a), APPROVED
2026-10-05**: the vendored fork is the production VMM path, and how it is
built, pinned and provisioned replaces the upstream release download.

## Context

Today every host gets Cloud Hypervisor from the upstream release:

- `infra/provision/versions.env` pins `CLOUD_HYPERVISOR_VERSION="v53.0"`;
- `infra/provision/common-system.sh` downloads
  `cloud-hypervisor/cloud-hypervisor/releases/download/${VERSION}/cloud-hypervisor-static[-aarch64]`
  to `/usr/local/bin/cloud-hypervisor`, accepts any binary whose `--version`
  contains the version string, and refuses a build without `--landlock`;
- `infra/lima/overdrive-dev.yaml` repeats the same download under its own
  `CLOUD_HYPERVISOR_VERSION` parameter;
- `infra/metal/native-preflight.sh` only checks that `cloud-hypervisor` is on
  `PATH`.

Nothing verifies the binary's content. A version-string match cannot tell the
upstream release from the fork, and the upstream release lacks the required
backend.

The fork is the submodule `vendors/cloud-hypervisor` (URL
`https://github.com/overdrive-sh/cloud-hypervisor.git`), branch
`overdrive/vhost-kernel-vsock` = v53.0 (`9ed824d6d`) + `9b68dbb57` +
`41a619d19`. Its gates passed: x86_64 clippy `-D warnings` (default and
`--no-default-features --features kvm`), the vsock and config unit tests, and
`fmt --check`; aarch64 compiled and passed clippy in Lima. Runtime evidence
exists for x86_64 only.

## Decision

1. **One source pin.** The production VMM is built from one full commit SHA of
   the fork. The submodule pointer is that SHA, and `versions.env` names it
   (`CLOUD_HYPERVISOR_FORK_REV`). The submodule is the build input; nothing
   builds from a moving branch.
2. **One release artifact per architecture.** A fork CI workflow builds
   static binaries for x86_64 and aarch64 from that SHA and publishes them as a
   release on `overdrive-sh/cloud-hypervisor`, tagged
   `v53.0-overdrive.<n>`. The tag, the asset name per architecture and each
   asset's SHA-256 are pinned in `versions.env`.
3. **Provisioning verifies content, not a version string.**
   `common-system.sh` and the Lima template download the pinned asset from the
   fork release, verify its SHA-256 against `versions.env` before installing,
   and refuse on mismatch. They install to `/usr/local/bin/cloud-hypervisor`
   as today. The upstream download is removed.
4. **Provisioning checks the capability it needs.** After install, provisioning
   refuses unless `cloud-hypervisor --help` lists both `--landlock` (today's
   check) and the `backend=vhost-kernel` vsock option.
5. **Runtime does not trust provisioning.** `Vmm::probe` repeats the capability
   check against the binary it will launch and refuses the node with a typed
   error (ADR-0146). Provisioning keeps a host from being built wrong; the
   probe keeps a wrongly built host from running.
6. **Lima stays a compile surface for the fork.** Lima installs the same
   pinned aarch64 asset so local tooling matches, but Lima remains non-signal
   for microVM boots (`.claude/rules/testing.md`).

Upstreaming the change to Cloud Hypervisor is outside this decision.

## Alternatives considered

- **Build from the submodule on every provisioned host** (`cargo build
  --release --locked`). Puts a Rust toolchain and a ~minutes build on every
  appliance host, and two hosts can produce different binaries from one SHA.
  Rejected.
- **Commit binaries into the repository.** Large binary blobs in git, and no
  provenance link to a CI build. Rejected.
- **Keep the upstream download and patch at install time.** Not a binary
  patch target. Rejected.
- **Pin only the tag.** A tag can be moved; a SHA-256 cannot. Rejected.

## Consequences

- `versions.env`'s `CLOUD_HYPERVISOR_VERSION` is replaced by the fork tag, its
  source SHA and per-architecture asset SHA-256s. `overdrive-dev.yaml`'s
  parameter is replaced the same way. The exact keys are pinned in the feature
  delta (§ *VMM fork provisioning contract*).
- Rebasing the fork (a new CH upstream release) is a deliberate change: new
  SHA, new tag, new checksums, and the V-1 evidence re-run on qualified metal.
- The fork's aarch64 binary has compile and lint evidence only.
- Any host image that ships Cloud Hypervisor consumes the same pinned
  artifact and checksum; nothing in this ADR depends on how such an image is
  assembled.
