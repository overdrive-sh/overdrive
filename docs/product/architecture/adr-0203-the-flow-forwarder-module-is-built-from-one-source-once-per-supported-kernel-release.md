# ADR-0203 — The flow-forwarder module is built from one in-repository source, once per supported kernel release

## Status

**Accepted — EVMOD-BUILD, approved by the user 2026-10-17.** GH #295. This
ADR decides only how the module is built: from which source, by which tool,
for which kernels. Who loads a built module is ADR-0205 (EVMOD-LOAD), and
which module reaches which Linux guest is ADR-0206 (EVMOD-GUEST-KIT); either
could be replaced while this build is kept. The source's placement and its
build tool are one decision here: the tool exists to build that source, and
neither is chosen without the other. It does not depend on ADR-0198 (the
module's lifetime). Follows from ADR-0197 (accepted 2026-10-16). The
contract is in the #295 feature delta, § *[REF] vsock data path — the
in-kernel event-driven forwarder (EVMOD)*, "Module placement and lifetime".

## Context

- **A module is bound to the kernel it was built against.** `finit_module`
  compares the module's `vermagic` with the kernel's through `same_magic`
  (`kernel/module/main.c:2593-2596 @ v7.0`, `ENOEXEC`). On a kernel built
  with `CONFIG_MODVERSIONS` that function is
  `kernel/module/version.c:97-105 @ v7.0`: for a module that carries CRCs it
  skips the first word — the kernel release — and compares the rest, and
  the kernel checks CRCs in its place: the CRC of `module_layout`
  (`version.c:75-94`; `main.c:3340-3341`, `ENOEXEC`) and the CRC of every
  symbol the module imports (`version.c:13-73`; `main.c:1261-1262`,
  `EINVAL`). Only on a kernel without `CONFIG_MODVERSIONS` is `same_magic`
  the `#else` stub that compares the whole string
  (`kernel/module/internal.h:421-424 @ v7.0`). The kernel therefore does not
  always compare the release; a module is built against the build tree of
  the release it runs on. The spikes ran a `7.0.0-38` host with `7.0.0-29`
  guests, so one source needs a build per kernel.
- **The module is C.** Rust-for-Linux on 7.0 has no socket abstractions
  (`vendors/linux/rust/kernel/net.rs:6`), and the module uses GPL-only
  symbols such as `tcp_abort` (`net/ipv4/tcp.c:5141 @ v7.0`).
- **The workspace's build rules.** No crate builds C kernel code. xtask is
  the home for tasks that consume kernel build artifacts and must not
  depend on any `overdrive-*` crate (`.claude/rules/development.md`, "xtask
  is build / test / dev orchestration").
- The kernels Overdrive runs: the pinned appliance kernel, host and guest
  (ADR-0068), and the stock Linux 7.0 kernels of the development and Tier-3
  hosts and the Tier-3 guest fixture.

## Decision

- **Source.** One C source tree, GPL-2.0, under `kmod/ovd_fwd/` at the
  repository root: the module, its frame parser in a file of its own
  (ADR-0201), and its UAPI header — the C SSOT of the control ABI and the
  frame constants, which the Rust adapter mirrors with layout and value
  assertions. No crate builds it.
- **Build.** `cargo xtask kmod-build --kernel-build-dir <dir>` runs the
  kernel's out-of-tree build against the build tree of one kernel release,
  `W=1`, `C=1` (sparse) and `checkpatch --strict` clean, and writes
  `target/kmod/<kernel-release>/ovd_fwd.ko` under the repository checkout
  it runs in, not under the cargo target directory, which several
  development workspaces can share. Kbuild's outputs go there too (`MO=`,
  `Makefile:137-139 @ v7.0`), so the source tree holds sources only.
- **Module identity.** `kmod-build` computes one SHA-256 digest over the
  files of `kmod/ovd_fwd/` that git does not ignore, using their
  working-tree contents (headers and `Kbuild` included). The file set is
  `git ls-files --cached --others --exclude-standard`: tracked files plus
  untracked files that are not ignored. A source file not yet added is
  therefore covered, so a build is never mistaken for another, and ignored
  stray files (`.DS_Store`, `*.swp`, both in the repository's `.gitignore`)
  are not. A stray file that is not ignored fails safe: the module reads as
  stale and is reloaded. The digest is compiled
  in as the module's `MODULE_VERSION`; the source does not build without
  it. A loaded module's identity is `/sys/module/ovd_fwd/version`
  (`kernel/module/main.c:607 @ v7.0`), which the lane setup and the test
  fixtures compare with their own build. The kernel's `srcversion` is not
  the identity: `modpost` digests the module's sources plus only those
  dependencies in the object's own directory
  (`scripts/mod/sumversion.c:366-367 @ v7.0`), so it misses the UAPI header
  under `include/` and the build file.
- **One build per supported kernel release.** Overdrive builds and
  publishes a module for each kernel it supports: the pinned appliance
  kernel and the stock 7.0 kernels its Tier-3 hosts and guest fixture run.
  The module is signed with the image's module-signing key when the kernel
  enforces module signatures.

## Alternatives considered

- **Built into the appliance kernel.** Considered in ADR-0198 (a modified
  kernel tree, which the #295 charter excludes; a second form for the stock
  Tier-3 kernels anyway).
- **A crate's `build.rs` builds the module.** It would put a kernel build
  tree on the path of every `cargo check` of that crate and drag the C
  toolchain into a Rust compile chain — the bootstrap failure the xtask rule
  exists to prevent. Rejected.
- **One module for every 7.0 release, loaded with forced version checks**
  (`MODULE_INIT_IGNORE_VERMAGIC`, honoured only under
  `CONFIG_MODULE_FORCE_LOAD`, `kernel/module/main.c:2585-2592`,
  `kernel/module/Kconfig:124 @ v7.0`). Loads a module against a layout it
  was not built for; the vermagic check is the kernel's protection against
  exactly that. Rejected.
- **DKMS on the target.** Builds on the machine that runs the module, which
  needs a compiler and kernel headers in the appliance image, against
  ADR-0068's immutable image. Rejected.

## Consequences

- Each supported kernel adds one module build to the release process; a
  kernel bump is a module rebuild.
- Any edit to a tracked file under `kmod/ovd_fwd/` changes the module's
  identity, so a development host's loaded module becomes stale. The lane
  setup reloads it while it is idle (ADR-0205; ADR-0198, as amended by the
  user's ruling MODULE-REFCOUNT of 2026-10-20). No reboot is needed.
- The C source is checked by the kernel's own tooling (`W=1`, sparse,
  `checkpatch`) in CI, and its ABI by the adapter's layout assertions and
  the probe's version check.
