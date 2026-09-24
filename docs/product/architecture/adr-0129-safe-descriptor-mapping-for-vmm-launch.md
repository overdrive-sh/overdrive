# ADR-0129 — The VMM child inherits exactly standard I/O and the TAP queue at descriptor 3

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R3. Proposed 2026-09-23 and revised 2026-09-24; reviewed by
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN.
Creation-time close-on-exec is outside this decision; the feature delta records
it as an implementation obligation.
Depends on ADR-0128. ADR-0143's launch seccomp filter is installed in this
decision's hook; ADR-0143 owns the filter, and this decision owns the hook and
its order. The file name names only part of the decision; the title is
authoritative. Exact mechanics live only in the #295 feature delta.

## Context

ADR-0128 requires the queue descriptor to appear in the Cloud Hypervisor child
at one fixed descriptor number, with no copy leaking to any other child. Four
facts shape the mechanism:

- Mapping a parent descriptor to a fixed child number requires a `dup2` in the
  child between fork and exec, which in Rust means a `pre_exec` closure. That
  closure is `unsafe`, and it runs where allocation and locks are not safe.
- Clearing close-on-exec on the parent's descriptor races every concurrent
  spawn in the process, such as another VM launch. The raced child would
  inherit another guest's queue.
- A mapping library alone does not close anything else. `command-fds` moves the
  mapped descriptor into place with `dup2` and clears close-on-exec only on the
  target; it "does not close other file descriptors". libvirt closes every
  other descriptor in the child (`virCommandMassClose`), and the Firecracker
  jailer closes every descriptor except standard input, output, and error.
  (Research:
  `docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
  Findings 1.4 and 1.5.)
- The `overdrive serve` process holds descriptors created without
  close-on-exec. The leg-F and leg-C listener sockets
  (`overdrive-worker/src/mtls_intercept.rs:340`, reached through
  `bind_transparent`), leg-S dial sockets
  (`overdrive-dataplane/src/mtls/mod.rs:662`), the netfilter and generic
  netlink sockets (`overdrive-netlink/src/nft.rs:1644`, `ethtool.rs:265`), the
  guest DNS socket (`overdrive-control-plane/src/dns_responder/responder.rs:414`),
  and the per-connection splice pipes (`overdrive-dataplane/src/mtls/splice.rs:533`,
  `:632`) are all inheritable today. A Cloud Hypervisor child launched while
  they exist inherits them. An inherited leg-F listener would let a
  compromised VMM accept connections meant for the transparent intercept.
- ADR-0143's seccomp filter must be loaded in the same child before its first
  exec, while the child is still single-threaded, so that it persists through
  the `prlimit` → `setpriv` → Cloud Hypervisor exec chain and binds every
  thread Cloud Hypervisor creates (spike increment-aa,
  `docs/feature/netns-density-295/spike/findings-tap-ioctl-seccomp.md`).

## Decision

The Cloud Hypervisor child inherits exactly descriptors 0, 1, and 2 and the
TAP queue at descriptor 3. Two mechanisms enforce it:

1. **Mapping.** The `command-fds` extension on `tokio::process::Command` maps
   the queue to child descriptor 3.
2. **In-child close.** One audited `pre_exec` hook, registered after the
   mapping, marks every descriptor above 3 close-on-exec with a single
   `close_range` call, so the exec closes all of them. Marking rather than
   closing keeps the standard library's own close-on-exec exec-error channel
   intact, so an exec failure still surfaces as a spawn error. A failure of the
   call fails the spawn.

The same hook is the one place the launch runs code in the child, and it is
where ADR-0143's launch seccomp filter is installed. Its order is fixed:

1. the `command-fds` mapping places the queue at descriptor 3 (its own hook,
   registered first);
2. `close_range` marks every descriptor above 3 close-on-exec;
3. the child sets `no_new_privs` and loads the ADR-0143 filter, as the hook's
   last effect;
4. the standard library execs the `prlimit` wrapper, which execs `setpriv`,
   which execs Cloud Hypervisor.

The mapping precedes the close so the queue already sits at descriptor 3 when
every higher descriptor is marked. The filter comes last, so every launcher
step before it runs unfiltered, and the child runs nothing of its own after
loading it except the exec. The filter is then in force from the first exec
onward, and every thread Cloud Hypervisor creates inherits it. A failure at any
step fails the spawn, and no exec happens.

The hook is registered on every Cloud Hypervisor launch. A launch that maps no
queue has no step 1, and its close marks every descriptor from 3 upward, so its
child inherits exactly descriptors 0, 1, and 2.

The in-child close is the guarantee. It covers every descriptor in the process,
including those that third-party or FFI code opens without close-on-exec.
Creating first-party descriptors close-on-exec, under a source gate, is a
separate hygiene obligation for every other spawn path. It is recorded in the
#295 feature delta and is not part of this decision.

## Alternatives considered

### `command-fds` alone

Rejected. It does not close other descriptors, so every inheritable descriptor
in the process reaches the VMM (Finding 1.4 and the inventory above).

### An in-crate `dup2` without `command-fds`

Rejected. `command-fds` already handles parent and child number collisions
without allocating in the child, and Android AVF's `virtmgr` uses it for the
same TAP handoff to crosvm (Finding 1.5). Re-implementing it adds audited
`unsafe` for no gain; the one in-child hook is the only `unsafe` the launch
needs.

### Close the descriptors in the child instead of marking them close-on-exec

Rejected. Closing every descriptor above 3 in the child would also close the
standard library's close-on-exec pipe that reports an exec failure. An exec
failure would then look like a successful spawn followed by a child exit.

### Rely on close-on-exec at creation only

Rejected. Descriptors opened by third-party or FFI code cannot be source-gated,
so creation-time discipline cannot be the guarantee.

### Pass the queue as the child's standard input

Rejected. It repurposes standard input as a network queue and can carry only
one descriptor, so it does not extend to multiqueue. Neither this nor
descriptor 3 has direct native evidence: both spikes passed descriptor 50 from a
blocking open. The production number and open flags must be proven natively
whichever is chosen.

### Clear close-on-exec in the parent

Rejected. It races concurrent spawns and leaks one guest's queue to another
child process.

## Consequences

Positive: the child's descriptor set is deterministic and complete, not a
by-product of every other component's hygiene. The dependency is well
maintained and Apache-2.0 licensed (version 0.3.3, published 2026-04-10, more
than five million downloads, 41 reverse dependencies, Google-owned), so its
review gate is closed. The `tokio` feature implements the extension for
`tokio::process::Command` through the same `pre_exec` + `dup2` child hook, read
from source in research addendum A7 (`command-fds` 0.3.3 `src/tokio.rs`);
DELIVER confirms it by compilation.

Negative:

- `overdrive-host` moves from `forbid(unsafe_code)` to `deny(unsafe_code)` with
  exactly one audited production exception: the function that registers the
  `pre_exec` hook. The hook issues three raw syscalls: `close_range` for this
  decision, and `prctl(PR_SET_NO_NEW_PRIVS)` and
  `seccomp(SECCOMP_SET_MODE_FILTER)` for ADR-0143. None allocates or takes a
  lock. The only other exception is test-only (feature delta).
- A new third-party dependency enters the security-sensitive VMM launch path.
- The confinement claim rests on native evidence that the child's descriptor
  table holds exactly standard I/O and its own queue at descriptor 3, with the
  production open flags.
- In one narrow race, the standard library's exec-error pipe can itself be
  numbered 3. This needs descriptor 3 to be freed by another thread after the
  queue opens and before `spawn` creates the pipe. The mapping then replaces
  the pipe in the child, and an exec failure is reported as an early VMM exit
  instead of a spawn error. Only the error classification degrades. The error
  bytes cannot reach the bridge: the TAP rejects a write shorter than its
  virtio-net header and is administratively down in any case. A successful
  exec is unaffected.
- `command-fds` 0.3.3's `dup2` in the child goes through `nix` 0.31.3
  `dup2_raw`, which does not check the `dup2` return before wrapping it in an
  `OwnedFd` (research addendum A7). A failed `dup2` therefore panics inside the
  forked child (`OwnedFd::from_raw_fd` panics on `-1`) rather than surfacing as
  a spawn `io::Error`, departing from the model that a hook failure fails the
  spawn. In a freshly forked single-threaded child mapping the queue onto
  descriptor 3, `dup2` has essentially no reachable failure path (`EBADF`
  cannot occur — the source is open; `EBUSY` needs a concurrent thread;
  `EMFILE` needs `RLIMIT_NOFILE <= 3`), so this is recorded as a property of the
  dependency chain, not a live defect.
