# ADR-0206 — A guest kernel and its forwarder module ship as one guest kit, and the guest refuses to start when they disagree

## Status

**Accepted — EVMOD-GUEST-KIT, approved by the user 2026-10-17, for Linux
guests.** GH #295. This ADR decides only which module a Linux guest's root
filesystem carries, and that the guest checks it. How modules are built is
ADR-0203 (EVMOD-BUILD); who loads them is ADR-0205 (EVMOD-LOAD). The release
check is part of this decision, not a second one: without it the kit rule is
a convention the kernel does not always enforce. A guest that runs no Linux
kernel carries no `ovd_fwd` module; its hop to the host-side frame protocol
(which is guest-neutral, ADR-0207) is
[GH #321](https://github.com/overdrive-sh/overdrive/issues/321). The guest's
error variants and their console text are in the #295 feature delta,
§ *[REF] vsock data path — the in-kernel event-driven forwarder (EVMOD)*,
"Module placement and lifetime", and § *[REF] Typed error taxonomy and who
branches*.

## Context

- **What the kernel checks at `finit_module`.** The first word of
  `vermagic` is `UTS_RELEASE` (`include/linux/vermagic.h:41-42 @ v7.0`), and
  `check_modinfo` compares the module's `vermagic` with the kernel's through
  `same_magic` (`kernel/module/main.c:2593-2596 @ v7.0`, `ENOEXEC`). Which
  `same_magic` runs depends on `CONFIG_MODVERSIONS`:
  - **With `CONFIG_MODVERSIONS`** it is `kernel/module/version.c:97-105 @
    v7.0`. For a module that carries CRCs it skips the first word — the
    release — and compares the rest, and the kernel checks CRCs instead:
    the CRC of `module_layout` (`version.c:75-94`, refused `ENOEXEC` at
    `main.c:3340-3341`) and the CRC of every symbol the module imports
    (`version.c:13-73`, refused `EINVAL` at `main.c:1261-1262`). A module
    built for another release therefore loads whenever every CRC it carries
    matches; the kernel never compares the release.
  - **Without `CONFIG_MODVERSIONS`** it is the `#else` stub that compares the
    whole string (`kernel/module/internal.h:421-424 @ v7.0`), so a module of
    another release is refused `ENOEXEC`.
  - The appliance image's kernel configuration does not exclude
    `CONFIG_MODVERSIONS`, so the kit rule cannot rest on the kernel's
    comparison.
- **The guest design.** The guest kernel is the pinned appliance kernel
  (ADR-0068 § 4), and guest root filesystems are built by the OCI-to-rootfs
  image factory (#259).
- **Today a VM spec names its own kernel and root filesystem** (BYO
  artifacts). That is an interim until the image factory (#259) delivers
  the guest image and `overdrive-init`, not the guest design: the platform ships `overdrive-init` and its contract, and the
  operator bakes `overdrive-init` into the root filesystem. A module built
  for one kernel cannot serve an operator's arbitrary kernel.
- **Who holds both facts.** Only the guest knows, before any flow, both the
  running kernel's release and the module's. The host's API ingress does
  not read the kernel image, and the host VM driver sees only an image
  file.

## Decision

- **A guest kernel and its module are one Overdrive artifact, the guest
  kit.** For each guest kernel Overdrive supports it publishes one kit: the
  kernel image, the module built against that kernel's build tree
  (ADR-0203), and the `overdrive-init` of the same release. The supported
  kits are the pinned appliance kernel's and, for Tier-3 only, the guest
  fixture's stock 7.0 kernel, whose module the fixture's provisioning
  builds on its host.
  - *The guest design (end state).* The image factory (#259) builds every
    guest root filesystem with the pinned kernel's kit `overdrive-init` and
    module. No deploy can pair a guest kernel with another kit's module.
  - *While VM specs name their own kernel and root filesystem (until
    #259).* A
    spec's kernel must be a kit's kernel, and its root filesystem carries
    that kit's `overdrive-init` and module — the contract under which the
    operator already bakes `overdrive-init` into the root filesystem. Any
    other guest kernel is unsupported.
- **The guest checks the kit before loading.** `overdrive-init` compares
  the module's `vermagic` release with the running release (`uname`)
  before `finit_module`, on every kernel configuration, and fails before
  READY with a cause that tells a missing module, a module of another
  release (naming both) and any other refusal apart. This check is the only
  comparison of the release on a kernel with `CONFIG_MODVERSIONS`. A module
  of the same release but another build is refused by the kernel — with
  `CONFIG_MODVERSIONS` by the rest of `vermagic` or a CRC, without it by the
  whole-string comparison — and reported as a load refusal; a
  same-release build whose every CRC matches loads, which is the kernel's
  own compatibility rule for the symbols it uses. No flow is admitted on a
  guest whose kit disagrees.

## Alternatives considered

- **Inject the module into the guest at launch** (a second initramfs or a
  virtio-fs share the VM driver supplies). The host would need a module
  store for every guest kernel release a spec might name, chosen from the
  kernel image's version string, a second boot medium on the VMM, and a
  host-side lookup on the boot path. The kit already binds the module to
  its kernel at build time, and the root filesystem already carries
  `overdrive-init` under a contract the operator follows. Rejected.
- **A digest allow-list of guest kernels, checked by the VM driver before
  launch.** The Tier-3 stock kernels differ per host, so the list cannot be
  a build constant, and it checks a proxy (the image's hash) where the guest
  checks the fact itself, with both releases in hand, before any flow.
  Rejected.
- **Refuse at deploy validation a VM spec whose kernel release has no
  published module.** The API ingress does not read the node's kernel
  image. Rejected.
- **No release check: rely on `finit_module`.** On a kernel with
  `CONFIG_MODVERSIONS` a module of another release loads whenever its CRCs
  match, so a kit mismatch may not be refused at all; where it is refused,
  it surfaces as `ENOEXEC` or `EINVAL` and the operator does not learn
  which two releases disagree. Rejected.

## Consequences

- A VM whose kernel and root filesystem do not come from one kit fails at
  start with a console message naming the cause, never at its first flow.
- While VM specs name their own artifacts (until #259), an operator's guest kernel
  must be a kit's kernel, in production the pinned appliance kernel; a
  kernel of the operator's choosing is not supported.
- Each supported guest kernel adds one kit to the release process.
- When #259 lands, the image factory builds every guest root filesystem
  from the pinned kernel's kit, and a VM spec no longer names a kernel.
