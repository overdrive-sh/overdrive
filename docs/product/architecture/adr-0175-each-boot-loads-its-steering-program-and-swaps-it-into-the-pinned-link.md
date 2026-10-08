# ADR-0175 — Each boot loads its own guest-prefix steering program and swaps it into the pinned link atomically; boot neither probes nor reads it back

## Status

**Proposed — decision D8a-LOAD-SWAP approved by user 2026-10-08; pending
independent DESIGN review.** GH #295. Recorded in the #295 feature delta,
§ *[REF] vsock Attachment Replacement DESIGN — PROPOSED 2026-10-05*; the exact
contract is § *Driven port — guest-prefix steering*. The steering mechanism is
ADR-0171; the route written after it is ADR-0152.

## Context

The guest-prefix steering link is pinned and outlives `serve` (ADR-0171). A
new `serve` binary must replace the program in that link with its own, and
must not leave the prefix locally delivered without a program deciding every
lookup at any step or after any crash.

The kernel offers `BPF_LINK_UPDATE`, which replaces a link's program in one
step and, when it fails, leaves the earlier program attached (assumption
K-L9, validated by V-26). The kernel verifier checks every program at load.

The steering program ships in the pinned image, built from the source whose
Tier-3 steering tests run in CI on the same pinned kernel (ADR-0068). The
formal model's sixth round showed that a read-back of the link after a
successful update is not load-bearing (finding r6-1).

## Decision

- **Load.** Every boot loads this binary's program for the configured prefix,
  with its own new, empty socket map; the kernel verifier checks it. A
  rejection refuses startup with nothing attached.
- **Swap.** Boot then swaps the program into the pinned link with
  `BPF_LINK_UPDATE`; with no pinned link it attaches a new link and pins it.
  The link is never detached. A failed or interrupted swap leaves the earlier
  program attached (on a first boot, no link), and boot refuses.
- **Then the route** (ADR-0152).
- **No runtime probe, no read-back.** The program's correctness is a property
  of the build, established by the Tier-3 steering tests in CI on the pinned
  kernel; after a successful update the link runs the new program by the
  kernel's own guarantee.
- **The map is not pinned.** The pinned link keeps the program and the
  program keeps its map. At boot the earlier map holds no live socket (the
  listeners closed with the earlier process), so nothing is lost.

## Alternatives considered

- **Probe the loaded program at every boot before the swap
  (`BPF_PROG_TEST_RUN`, or a link in a private namespace with real connects)
  and read the link back after it.** Rechecks on the node a property CI
  establishes for the same source on the same pinned kernel, adds a kernel
  assumption of its own (that a test run decides as the attached program
  would), and the read-back is not load-bearing (r6-1). Rejected by the user.
- **Fence the prefix with a `prohibit` route around the swap.** Keeps the
  fence, its write and its failure path, and turns every binary upgrade into
  a refused window, to cover a state — `local` over the newly swapped program
  — that exposes nothing when the shipped program is correct. Rejected.
- **Detach the earlier link and attach a new one.** Opens a window in which
  the prefix is `local` with no lookup decision. Rejected.
- **Pin the map and reuse it across boots.** Binds every binary's program to
  the previous binary's map layout and leaves a second pinned object to
  converge, for no gain. Rejected.

## Consequences

- Every binary upgrade replaces the steering program atomically, with no
  refused window; a boot refused at the steering leaves the node as the
  earlier boot left it, and every lookup to the prefix drops while `serve` is
  down.
- A logic defect in the steering program is a failed build, not a boot
  refusal (feature delta A-33).
- A falsified K-L9 (V-26 (viii)) returns this decision to DESIGN.
- The order is checked by the formal model (ADR-0168, module `prefix_boot`).
