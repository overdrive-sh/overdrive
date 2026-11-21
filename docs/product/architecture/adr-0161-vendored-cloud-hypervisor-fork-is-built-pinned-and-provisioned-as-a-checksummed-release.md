# ADR-0161 — The vendored Cloud Hypervisor fork is built from a pinned commit and provisioned as a checksum-verified release artifact

## Status

**Accepted — decision D21 approved by user 2026-10-05; RESET-EXIT (a guest
reset ends the VMM, ADR-0182) approved by user 2026-10-11, its mechanism
amended 2026-11-20 to the launcher flag, see § Amendment.** GH #295. Recorded in the #295 feature delta,
§ *[REF] vsock Attachment Replacement DESIGN*.

It implements the provisioning half of ruling **D13 = (a), APPROVED
2026-10-05**: the vendored fork is the only VMM Overdrive runs with, and this
ADR fixes how it is built, pinned and provisioned.

## Amendment — 2026-11-20: the fork is upstream `main` plus our commits

User ruling: the less our fork differs from upstream the better. The fork
branch is now `main` (upstream Cloud Hypervisor `main`, v56, plus our commits
on top), no longer `overdrive/vhost-kernel-vsock` (v53.0 plus two commits).
The fork no longer carries a guest-reset change. It keeps upstream's
`--guest-reset-action` default and the launcher passes `exit-vmm` on every
launch (ADR-0182). The fork now differs from upstream only by the vhost-kernel
vsock device (ADR-0146), its backend flag, and the O_EXCL block-device open
(F2, ADR-0310 family). F1 (fsync propagation) and F3 (explicit `image_type`)
of the volume work are upstream's own now (commits `91e4be988`, `8e9c08689`,
`1f468d822`, `f2cedec65`). Where the Context and Decision below say the pinned
fork commit carries the reset-exit change, or name the old branch and commit
list, they describe the first-accepted pin: the one-pinned-SHA, release-artifact
and checksum decisions stand, and the SHA, tag and checksums pinned in
`versions.env` are those of the build from the new branch.

## Context

Today every host downloads Cloud Hypervisor from the `cloud-hypervisor`
project's GitHub releases:

- `infra/provision/versions.env` pins `CLOUD_HYPERVISOR_VERSION="v53.0"`;
- `infra/provision/common-system.sh` downloads
  `cloud-hypervisor/cloud-hypervisor/releases/download/${VERSION}/cloud-hypervisor-static[-aarch64]`
  to `/usr/local/bin/cloud-hypervisor`, accepts any binary whose `--version`
  contains the version string, and refuses a build without `--landlock`;
- `infra/lima/overdrive-dev.yaml` repeats the same download under its own
  `CLOUD_HYPERVISOR_VERSION` parameter;
- `infra/metal/native-preflight.sh` only checks that `cloud-hypervisor` is on
  `PATH`.

Nothing verifies the binary's content, and Overdrive requires the fork's
`backend=vhost-kernel` vsock device (ADR-0146); the end of the VMM on a guest
reset is the launcher's flag on upstream's option (ADR-0182), not a fork
property. The binary must be the pinned fork build, byte for byte.

The fork is the submodule `vendors/cloud-hypervisor` (URL
`https://github.com/overdrive-sh/cloud-hypervisor.git`), branch
`overdrive/vhost-kernel-vsock` = v53.0 (`9ed824d6d`) + `9b68dbb57` +
`41a619d19`. Those commits' gates passed: x86_64 clippy `-D warnings`
(default and `--no-default-features --features kvm`), the vsock and config
unit tests, and `fmt --check`; aarch64 compiled and passed clippy in Lima.
Runtime evidence exists for x86_64 only. (As first accepted; the branch is now
`main`, see § Amendment. The reset-exit change of ADR-0182 was never a commit
on the old branch and is now the launcher's flag.)

## Decision

1. **One source pin.** The production VMM is built from one full commit SHA of
   the fork, and that commit carries the `backend=vhost-kernel` vsock device
   (guest reset needs no fork commit, ADR-0182). The submodule pointer is that
   SHA, and `versions.env` names it (`CLOUD_HYPERVISOR_FORK_REV`). The
   submodule is the build input; nothing builds from a moving branch.
2. **One release artifact per architecture.** A fork CI workflow builds
   static binaries for x86_64 and aarch64 from that SHA and publishes them as a
   release on `overdrive-sh/cloud-hypervisor`, tagged
   `v53.0-overdrive.<n>`. The tag, the asset name per architecture and each
   asset's SHA-256 are pinned in `versions.env`.
3. **Provisioning verifies content, not a version string.**
   `common-system.sh` and the Lima template download the pinned asset from the
   fork release, verify its SHA-256 against `versions.env` before installing,
   and refuse on mismatch. They install to `/usr/local/bin/cloud-hypervisor`
   as today. The previous download source is removed. The checksum is the
   identity check: a host carries the pinned fork build or provisioning fails.
4. **No runtime binary identification.** Overdrive runs only with the vendored
   fork. Neither provisioning nor `Vmm::probe` inspects the installed binary to
   tell the fork from any other Cloud Hypervisor build; no typed error exists
   for "a different build is installed". `Vmm::probe` keeps its existing
   stages (including the `--landlock` check). No startup probe opens
   `/dev/vhost-vsock`: the VMM opens it at launch, and a launch that cannot
   fails pre-READY through the typed VMM-exit start failure (ADR-0146).
5. **Lima stays a compile surface for the fork.** Lima installs the same
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
- **Keep the existing download and patch at install time.** Not a binary
  patch target. Rejected.
- **Pin only the tag.** A tag can be moved; a SHA-256 cannot. Rejected.
- **Detect a non-fork binary at runtime (`--help` capability probe, typed
  refusal).** Overdrive is only ever run with the vendored fork, and the
  checksum already pins its identity; a runtime probe for an unsupported
  configuration adds a stage, an error variant and tests that defend nothing.
  Rejected.

## Consequences

- `versions.env`'s `CLOUD_HYPERVISOR_VERSION` is replaced by the fork tag, its
  source SHA and per-architecture asset SHA-256s. `overdrive-dev.yaml`'s
  parameter is replaced the same way. The exact keys are pinned in the feature
  delta (§ *VMM fork provisioning contract*).
- Rebasing the fork onto a new Cloud Hypervisor version is a deliberate change: new
  SHA, new tag, new checksums, our remaining fork commits carried forward
  (nothing for guest reset), and the V-1 evidence and a real-VM guest-reset
  check (ADR-0182) re-run on qualified metal.
- The fork's aarch64 binary has compile and lint evidence only.
- Any host image that ships Cloud Hypervisor consumes the same pinned
  artifact and checksum; nothing in this ADR depends on how such an image is
  assembled.
